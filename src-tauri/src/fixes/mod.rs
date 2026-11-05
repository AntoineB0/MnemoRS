use crate::storage::Db;

/// Commandes qui ne constituent ni un problème ni une correction.
const NOISE: &[&str] = &["ls", "clear", "history", "pwd", "echo", "which", "type"];

const WEIGHT_SAME_CWD: f32 = 0.40;
const WEIGHT_PROXIMITY: f32 = 0.30;
const WEIGHT_SAME_TOOL: f32 = 0.20;
const WEIGHT_SHARED_TOKEN: f32 = 0.10;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixSettings {
    pub window_minutes: u32,
    pub auto_threshold: f32,
    pub suggest_threshold: f32,
}

impl Default for FixSettings {
    fn default() -> Self {
        Self {
            window_minutes: 10,
            auto_threshold: 0.85,
            suggest_threshold: 0.50,
        }
    }
}

impl FixSettings {
    pub fn window_secs(&self) -> i64 {
        self.window_minutes.max(1) as i64 * 60
    }
}

/// Score de confiance d'un couple échec → correction.
pub fn score(failed: &str, fix: &str, same_cwd: bool, delay_secs: i64, window_secs: i64) -> f32 {
    let mut score = 0.0;

    if same_cwd {
        score += WEIGHT_SAME_CWD;
    }

    let ratio = (delay_secs.max(0) as f32 / window_secs.max(1) as f32).clamp(0.0, 1.0);
    score += WEIGHT_PROXIMITY * (1.0 - ratio);

    let failed_tokens = tokens(failed);
    let fix_tokens = tokens(fix);
    let same_tool = failed_tokens.first().is_some_and(|tool| fix_tokens.first() == Some(tool));
    if same_tool {
        score += WEIGHT_SAME_TOOL;
    }
    if failed_tokens
        .iter()
        .skip(1)
        .any(|token| fix_tokens.iter().any(|other| other == token))
    {
        score += WEIGHT_SHARED_TOKEN;
    }

    score.clamp(0.0, 1.0)
}

/// Statut à retenir, ou `None` si le couple est trop incertain pour être gardé.
pub fn classify(score: f32, settings: &FixSettings) -> Option<&'static str> {
    if score >= settings.auto_threshold {
        Some("auto")
    } else if score >= settings.suggest_threshold {
        Some("suggested")
    } else {
        None
    }
}

pub fn is_noise(cmd: &str) -> bool {
    let first = cmd.split_whitespace().next().unwrap_or("");
    let name = first.rsplit('/').next().unwrap_or(first);
    NOISE.contains(&name)
}

/// Le couple vaut-il la peine d'être étudié ? (échec réel, correction réelle)
pub fn is_candidate_pair(failed: &str, fix: &str) -> bool {
    !is_noise(failed) && !is_noise(fix) && failed.trim() != fix.trim()
}

fn tokens(cmd: &str) -> Vec<String> {
    cmd.split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

/// Échec en attente de correction.
#[derive(Debug, Clone)]
pub struct PendingFix {
    pub id: i64,
    pub cmd: String,
    pub cwd: Option<String>,
    pub ts: i64,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Commande de bruit : on garde l'échec en attente pour la vraie correction.
    Keep,
    /// Ce n'est pas une correction de cet échec.
    Drop,
    Fix { confidence: f32, status: &'static str },
}

/// Décide du sort d'un échec en attente face à une commande réussie.
pub fn evaluate(
    pending: &PendingFix,
    cmd: &str,
    cwd: Option<&str>,
    exit_code: i32,
    ts: i64,
    settings: &FixSettings,
) -> Decision {
    if exit_code != 0 {
        return Decision::Drop;
    }
    if is_noise(cmd) {
        return Decision::Keep;
    }
    if !is_candidate_pair(&pending.cmd, cmd) || pending.cwd.as_deref() != cwd {
        return Decision::Drop;
    }

    let delay = ts - pending.ts;
    if delay > settings.window_secs() * 1000 {
        return Decision::Drop;
    }

    let confidence = score(
        &pending.cmd,
        cmd,
        true,
        delay / 1000,
        settings.window_secs(),
    );
    match classify(confidence, settings) {
        Some(status) => Decision::Fix {
            confidence,
            status,
        },
        None => Decision::Drop,
    }
}

pub fn settings(db: &Db) -> FixSettings {
    let defaults = FixSettings::default();
    let number = |key: &str| db.get_setting(key).ok().flatten().and_then(|v| v.parse::<f32>().ok());

    FixSettings {
        window_minutes: number("fix_window_minutes")
            .map(|v| v as u32)
            .unwrap_or(defaults.window_minutes)
            .clamp(1, 1440),
        auto_threshold: number("fix_auto_threshold")
            .unwrap_or(defaults.auto_threshold)
            .clamp(0.0, 1.0),
        suggest_threshold: number("fix_suggest_threshold")
            .unwrap_or(defaults.suggest_threshold)
            .clamp(0.0, 1.0),
    }
}

/// Rejoue l'heuristique sur l'historique existant, une seule fois.
/// Seules les certitudes sont conservées : les cas douteux ne sont pas créés,
/// pour ne pas noyer la file de validation.
pub fn backfill(db: &Db, limit: usize) -> anyhow::Result<usize> {
    if db.get_setting("fixes_backfilled")?.is_some() {
        return Ok(0);
    }

    let settings = settings(db);
    let rows = db.backfill_commands(limit)?;
    let mut pending: Option<PendingFix> = None;
    let mut session: Option<&str> = None;
    let mut inserted = 0;

    for row in &rows {
        if session != row.session_id.as_deref() {
            pending = None;
            session = row.session_id.as_deref();
        }

        match row.exit_code {
            Some(code) if code != 0 => {
                if !is_noise(&row.cmd) {
                    pending = Some(PendingFix {
                        id: row.id,
                        cmd: row.cmd.clone(),
                        cwd: row.cwd.clone(),
                        ts: row.ts,
                    });
                }
            }
            Some(0) => {
                let Some(failed) = pending.take() else {
                    continue;
                };
                match evaluate(
                    &failed,
                    &row.cmd,
                    row.cwd.as_deref(),
                    0,
                    row.ts,
                    &settings,
                ) {
                    Decision::Keep => pending = Some(failed),
                    // Le rattrapage ne conserve que les certitudes.
                    Decision::Fix { confidence, status } if status == "auto" => {
                        db.insert_fix(failed.id, row.id, confidence, "auto", "backfill", row.ts)?;
                        inserted += 1;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    db.set_setting("fixes_backfilled", "1")?;
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: i64 = 600;

    #[test]
    fn meme_outil_immediat_est_auto() {
        let value = score("tar -xzf a.tgz", "tar -xJf a.tar.xz", true, 5, WINDOW);
        assert!(value > 0.85, "score {value}");
        assert_eq!(classify(value, &FixSettings::default()), Some("auto"));
    }

    #[test]
    fn outil_different_reste_en_validation() {
        let value = score("python app.py", "pip install requests", true, 30, WINDOW);
        assert!((0.5..0.85).contains(&value), "score {value}");
        assert_eq!(classify(value, &FixSettings::default()), Some("suggested"));
    }

    #[test]
    fn dossier_different_penalise() {
        let same = score("make", "make install", true, 10, WINDOW);
        let other = score("make", "make install", false, 10, WINDOW);
        assert!(other < same);
    }

    #[test]
    fn hors_fenetre_tombe_sous_le_seuil() {
        let edge = score("make", "cd build", true, WINDOW, WINDOW);
        assert_eq!(classify(edge, &FixSettings::default()), None);

        let other_dir = score("make", "cd build", false, WINDOW, WINDOW);
        assert_eq!(classify(other_dir, &FixSettings::default()), None);

        let same_tool = score("make", "make install", true, WINDOW, WINDOW);
        assert_eq!(classify(same_tool, &FixSettings::default()), Some("suggested"));
    }

    #[test]
    fn bruit_reconnu() {
        assert!(is_noise("ls -la"));
        assert!(is_noise("/usr/bin/clear"));
        assert!(!is_noise("tar -xzf a.tgz"));
        assert!(!is_candidate_pair("ls", "ls -la"));
        assert!(!is_candidate_pair("make", "make"));
        assert!(is_candidate_pair("make", "make install"));
    }

    #[test]
    fn rattrapage_ne_garde_que_les_certitudes() {
        use crate::capture::CommandRecord;
        use crate::storage::Db;

        let db = Db::open_in_memory().unwrap();
        db.ensure_session("s", "zsh", Some("/tmp"), 1).unwrap();

        let record = |cmd: &str, code: i32| CommandRecord {
            cmd: cmd.into(),
            cwd: Some("/tmp".into()),
            exit_code: code,
            output: String::new(),
            truncated: false,
            sensitive: false,
            duration_ms: 1,
        };

        let tar_failed = db
            .insert_command("s", &record("tar -xzf a.tgz", 2), 1_000)
            .unwrap();
        db.insert_command("s", &record("tar -xJf a.tar.xz", 0), 2_000)
            .unwrap();

        let py_failed = db
            .insert_command("s", &record("python app.py", 1), 3_000)
            .unwrap();
        db.insert_command("s", &record("pip install requests", 0), 4_000)
            .unwrap();

        let count = backfill(&db, 100).unwrap();
        assert_eq!(count, 1, "seule la correction certaine est conservée");

        let fix = db.fix_for_command(tar_failed).unwrap().unwrap();
        assert_eq!(fix.status, "auto");
        assert!(fix.confidence >= 0.85);

        assert!(db.fix_for_command(py_failed).unwrap().is_none());
        assert!(db.pending_fixes(10).unwrap().is_empty());

        // le drapeau empêche une seconde passe
        assert_eq!(backfill(&db, 100).unwrap(), 0);
    }

    #[test]
    fn bruit_reussi_garde_l_echec_en_attente() {
        let settings = FixSettings::default();
        let pending = PendingFix {
            id: 1,
            cmd: "tar -xzf a.tgz".into(),
            cwd: Some("/tmp".into()),
            ts: 1_000,
        };

        assert_eq!(
            evaluate(&pending, "ls", Some("/tmp"), 0, 2_000, &settings),
            Decision::Keep
        );
        assert_eq!(
            evaluate(&pending, "tar -xzf a.tgz", Some("/tmp"), 0, 2_000, &settings),
            Decision::Drop
        );
        assert!(matches!(
            evaluate(&pending, "cd /tmp", Some("/tmp"), 0, 2_000, &settings),
            Decision::Fix { status: "suggested", .. }
        ));
        assert!(matches!(
            evaluate(&pending, "tar -xJf a.tar.xz", Some("/tmp"), 0, 2_000, &settings),
            Decision::Fix { status: "auto", .. }
        ));
        assert_eq!(
            evaluate(&pending, "tar -xJf a.tar.xz", Some("/autre"), 0, 2_000, &settings),
            Decision::Drop
        );
    }

    #[test]
    fn seuils_reglables() {
        let strict = FixSettings {
            auto_threshold: 0.95,
            ..Default::default()
        };
        let value = score("git push", "git pull", true, 10, WINDOW);
        assert_eq!(classify(value, &FixSettings::default()), Some("auto"));
        assert_eq!(classify(value, &strict), Some("suggested"));
    }
}

use serde::Deserialize;

/// Commande passée telle qu'injectée dans le contexte du modèle.
#[derive(Debug, Clone, Deserialize)]
pub struct CommandContext {
    pub cmd: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub output: String,
}

/// Extrait de documentation retrouvé par la recherche hybride.
#[derive(Debug, Clone)]
pub struct DocPassage {
    pub source: String,
    pub section: Option<String>,
    pub content: String,
    pub deleted: bool,
}

/// Correction déjà utilisée par le passé pour une commande similaire.
#[derive(Debug, Clone)]
pub struct FixPassage {
    pub failed: String,
    pub fix: String,
    pub cwd: Option<String>,
}

pub struct ContextInput<'a> {
    pub cwd: Option<&'a str>,
    pub shell: &'a str,
    pub recent: Vec<CommandContext>,
    pub failures: Vec<CommandContext>,
    pub docs: Vec<DocPassage>,
    pub fixes: Vec<FixPassage>,
}

/// Budget approximatif en caractères (aucun tokenizer : ~4 caractères/token).
/// Ollama démarre à 4096 tokens de contexte : on garde de la place pour la
/// réponse en visant ~1500 tokens de prompt.
const BUDGET_CHARS: usize = 6_000;
/// Une sortie de commande réussie sert rarement en entier ; la couper laisse
/// la place à la documentation et à plus de commandes. Les échecs gardent
/// leur sortie complète : c'est là que se trouve le message d'erreur.
const SUCCESS_OUTPUT_CHARS: usize = 300;
const OUTPUT_CHARS: usize = 1_500;
const FAILURE_OUTPUT_CHARS: usize = 800;

const SYSTEM: &str = "Tu es l'assistant intégré à MnemoRS, un terminal. \
Tu aides l'utilisateur à comprendre et corriger les commandes qu'il exécute. \
Le contexte ci-dessous provient de son historique réel : commandes, dossier, \
code de retour et sortie. Appuie-toi dessus et cite les éléments précis \
(commande, code de retour, message d'erreur). N'invente jamais un code de \
retour, une commande ou une sortie absents du contexte ; si l'information \
manque, dis-le. Réponds uniquement en français, sans mélanger d'anglais, de façon \
concise, et propose des commandes en bloc de code quand c'est utile.";

/// Assemble le prompt système : en-tête, documentation puis commandes
/// récentes et échecs, en supprimant les éléments les plus anciens quand le
/// budget est dépassé. La documentation passe en premier : sinon un historique
/// chargé la fait disparaître entièrement.
pub fn build(input: &ContextInput) -> String {
    let mut out = String::from(SYSTEM);

    out.push_str("\n\n## Session");
    out.push_str(&format!(
        "\n- Dossier courant : {}",
        input.cwd.unwrap_or("inconnu")
    ));
    out.push_str(&format!("\n- Shell : {}", input.shell));

    let mut used = out.len();

    if !input.docs.is_empty() {
        out.push_str("\n## Documentation\n");
        out.push_str("Extraits de manuels et READMEs retrouvés pour la question. Cite la source quand tu t'en sers.\n");
        for doc in &input.docs {
            let cost = doc.content.len() + 120;
            if used + cost > BUDGET_CHARS {
                continue;
            }
            used += cost;
            out.push_str(&format!("\n[{}]\n", doc_label(doc)));
            out.push_str(&doc.content);
            out.push('\n');
        }
    }

    if input.recent.is_empty() {
        out.push_str("\n\nAucune commande enregistrée dans cette session.");
    } else {
        out.push_str("\n\n## Commandes récentes");
        out.push_str("\nDe la plus ancienne à la plus récente.\n");

        let mut selected: Vec<(&CommandContext, usize)> = Vec::new();
        for command in input.recent.iter().rev() {
            let cap = if command.exit_code == Some(0) {
                SUCCESS_OUTPUT_CHARS
            } else {
                OUTPUT_CHARS
            };
            let cost = estimate(command, cap);
            if used + cost > BUDGET_CHARS {
                continue;
            }
            used += cost;
            selected.push((command, cap));
        }
        selected.reverse();

        if selected.is_empty() {
            out.push_str("\n(historique trop volumineux pour le budget)\n");
        }
        for (command, cap) in selected {
            out.push_str(&format_command(command, cap));
        }
    }

    if !input.fixes.is_empty() {
        out.push_str("\n## Corrections déjà utilisées\n");
        out.push_str("Ces corrections ont fonctionné par la suite dans des situations similaires.\n");
        for fix in &input.fixes {
            let cost = fix.failed.len() + fix.fix.len() + 80;
            if used + cost > BUDGET_CHARS {
                break;
            }
            used += cost;
            out.push_str(&format!("\n```\n$ {}\n```\n", fix.failed.trim()));
            if let Some(cwd) = &fix.cwd {
                out.push_str(&format!("(dossier {cwd})\n"));
            }
            out.push_str(&format!("```\n$ {}\n```\n", fix.fix.trim()));
        }
    }

    if !input.failures.is_empty() {
        out.push_str("\n## Échecs récents");
        out.push_str("\nCommandes qui ont échoué, pour repérer les erreurs récurrentes.\n");
        for command in &input.failures {
            let cost = estimate(command, FAILURE_OUTPUT_CHARS);
            if used + cost > BUDGET_CHARS {
                break;
            }
            used += cost;
            out.push_str(&format_command(command, FAILURE_OUTPUT_CHARS));
        }
    }

    out
}

fn doc_label(doc: &DocPassage) -> String {
    let base = match &doc.section {
        Some(section) => format!("{} § {}", doc.source, section),
        None => doc.source.clone(),
    };
    if doc.deleted {
        format!("{base} — document supprimé depuis l'indexation")
    } else {
        base
    }
}

fn estimate(command: &CommandContext, output_chars: usize) -> usize {
    command.cmd.len() + command.output.len().min(output_chars) + 80
}

fn format_command(command: &CommandContext, output_chars: usize) -> String {
    let mut out = String::from("\n```\n$ ");
    out.push_str(command.cmd.trim());
    out.push_str("\n```\n");

    let mut meta = Vec::new();
    if let Some(cwd) = &command.cwd {
        meta.push(format!("dossier {cwd}"));
    }
    if let Some(code) = command.exit_code {
        meta.push(format!("code {code}"));
    }
    if !meta.is_empty() {
        out.push_str(&format!("({})\n", meta.join(", ")));
    }

    let output = command.output.trim();
    if output.is_empty() {
        out.push_str("(aucune sortie)\n");
    } else {
        out.push_str("```\n");
        out.push_str(&truncate(output, output_chars));
        out.push_str("\n```\n");
    }
    out
}

fn truncate(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    let head: String = input.chars().take(max * 2 / 3).collect();
    let tail: String = input
        .chars()
        .skip(input.chars().count() - max / 3)
        .collect();
    format!("{head}\n…[tronqué]…\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(cmd: &str, output: &str, exit_code: i32) -> CommandContext {
        CommandContext {
            cmd: cmd.into(),
            cwd: Some("/home/moi".into()),
            exit_code: Some(exit_code),
            output: output.into(),
        }
    }

    fn input(recent: Vec<CommandContext>, failures: Vec<CommandContext>) -> ContextInput<'static> {
        ContextInput {
            cwd: Some("/home/moi/projet"),
            shell: "zsh",
            recent,
            failures,
            docs: Vec::new(),
            fixes: Vec::new(),
        }
    }

    #[test]
    fn contient_dossier_shell_et_commande() {
        let prompt = build(&input(vec![command("ls", "fichier", 0)], vec![]));
        assert!(prompt.contains("/home/moi/projet"));
        assert!(prompt.contains("zsh"));
        assert!(prompt.contains("$ ls"));
        assert!(prompt.contains("code 0"));
        assert!(prompt.contains("fichier"));
    }

    #[test]
    fn historique_vide_reste_valide() {
        let prompt = build(&input(vec![], vec![]));
        assert!(prompt.contains("Aucune commande"));
        assert!(prompt.contains("N'invente jamais"));
    }

    #[test]
    fn ordre_chronologique_conserve() {
        let prompt = build(&input(
            vec![
                command("premiere", "", 0),
                command("deuxieme", "", 0),
                command("troisieme", "", 0),
            ],
            vec![],
        ));
        let a = prompt.find("premiere").unwrap();
        let b = prompt.find("deuxieme").unwrap();
        let c = prompt.find("troisieme").unwrap();
        assert!(a < b && b < c);
    }

    #[test]
    fn sortie_longue_tronquee() {
        let long = "x".repeat(10_000);
        let prompt = build(&input(vec![command("cat gros", &long, 0)], vec![]));
        assert!(prompt.contains("[tronqué]"));
        assert!(prompt.len() < 10_000);
    }

    #[test]
    fn budget_respecte_en_gardant_les_plus_recentes() {
        let mut recent = Vec::new();
        for i in 0..500 {
            recent.push(command(&format!("commande-{i}"), &"y".repeat(1000), 0));
        }
        let prompt = build(&input(recent, vec![]));
        assert!(prompt.len() <= BUDGET_CHARS + 2000, "taille {}", prompt.len());
        assert!(prompt.contains("commande-499"));
        assert!(!prompt.contains("commande-0"));
    }

    #[test]
    fn echecs_inclus_et_tronques_plus_court() {
        let prompt = build(&input(
            vec![command("true", "", 0)],
            vec![command("false", &"z".repeat(5000), 1)],
        ));
        assert!(prompt.contains("Échecs récents"));
        assert!(prompt.contains("$ false"));
        assert!(prompt.contains("code 1"));
    }

    #[test]
    fn documentation_injectee_avec_sa_source() {
        let mut context = input(vec![], vec![]);
        context.docs = vec![DocPassage {
            source: "doc:tar".into(),
            section: Some("OPTIONS".into()),
            content: "utiliser -x pour extraire".into(),
            deleted: false,
        }];
        let prompt = build(&context);
        assert!(prompt.contains("## Documentation"));
        assert!(prompt.contains("doc:tar § OPTIONS"));
        assert!(prompt.contains("utiliser -x pour extraire"));
    }

    #[test]
    fn document_supprime_signale_dans_le_prompt() {
        let mut context = input(vec![], vec![]);
        context.docs = vec![DocPassage {
            source: "/home/moi/README.md".into(),
            section: Some("Installation".into()),
            content: "npm install".into(),
            deleted: true,
        }];
        let prompt = build(&context);
        assert!(prompt.contains("document supprimé"));
    }

    #[test]
    fn documentation_respecte_le_budget() {
        let mut context = input(vec![], vec![]);
        context.docs = (0..50)
            .map(|index| DocPassage {
                source: format!("doc:{index}"),
                section: None,
                content: "z".repeat(2000),
                deleted: false,
            })
            .collect();
        let prompt = build(&context);
        assert!(prompt.len() <= BUDGET_CHARS + 2000);
        assert!(prompt.contains("doc:0"));
        assert!(!prompt.contains("doc:49"));
    }

    #[test]
    fn documentation_garde_sa_place_malgre_un_historique_plein() {
        let mut context = input(
            (0..20)
                .map(|index| command(&format!("commande-{index}"), &"y".repeat(1500), 0))
                .collect(),
            vec![],
        );
        context.docs = vec![DocPassage {
            source: "/home/moi/RsHash/README.md".into(),
            section: Some("Installation".into()),
            content: "maturin develop".into(),
            deleted: false,
        }];
        let prompt = build(&context);
        assert!(prompt.contains("## Documentation"));
        assert!(prompt.contains("maturin develop"));
        assert!(prompt.contains("commande-19"));
    }

    #[test]
    fn sortie_reussie_tronquee_avant_celle_d_un_echec() {
        let prompt = build(&input(
            vec![command("succes", &"s".repeat(1000), 0)],
            vec![command("echec", &"e".repeat(400), 1)],
        ));
        assert!(prompt.contains("[tronqué]"));
        assert!(prompt.contains(&"e".repeat(400)));
    }

    #[test]
    fn corrections_passees_injectees() {
        let mut context = input(vec![command("tar -xzf a.tgz", "", 2)], vec![]);
        context.fixes = vec![FixPassage {
            failed: "tar -xzf a.tgz".into(),
            fix: "tar -xJf a.tar.xz".into(),
            cwd: Some("/home/moi".into()),
        }];
        let prompt = build(&context);
        assert!(prompt.contains("## Corrections déjà utilisées"));
        assert!(prompt.contains("$ tar -xzf a.tgz"));
        assert!(prompt.contains("$ tar -xJf a.tar.xz"));
    }

    #[test]
    fn sortie_vide_est_signalee() {
        let prompt = build(&input(vec![command("false", "", 1)], vec![]));
        assert!(prompt.contains("(aucune sortie)"));
    }
}

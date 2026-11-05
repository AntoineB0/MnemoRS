use std::path::Path;
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::capture::CommandRecord;
use crate::providers::ProviderConfig;

const MIGRATIONS: &[&str] = &[
    include_str!("schema.sql"),
    include_str!("providers.sql"),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRow {
    pub id: i64,
    pub cmd: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub output_truncated: bool,
    pub sensitive: bool,
    pub duration_ms: Option<i64>,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDetail {
    #[serde(flatten)]
    pub row: CommandRow,
    pub output: String,
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("ouverture de {}", path.display()))?;
        Self::from_connection(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn ensure_session(&self, id: &str, shell: &str, cwd: Option<&str>, ts: i64) -> Result<()> {
        self.conn()?.execute(
            "INSERT OR IGNORE INTO sessions (id, shell, started_ts, cwd_start) VALUES (?1, ?2, ?3, ?4)",
            params![id, shell, ts, cwd],
        )?;
        Ok(())
    }

    pub fn insert_command(
        &self,
        session_id: &str,
        record: &CommandRecord,
        ts: i64,
    ) -> Result<i64> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO commands
               (session_id, cmd, cwd, exit_code, output, output_truncated, sensitive, duration_ms, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                session_id,
                record.cmd,
                record.cwd,
                record.exit_code,
                record.output,
                record.truncated,
                record.sensitive,
                record.duration_ms as i64,
                ts,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn search(&self, query: &str, limit: usize, only_failed: bool) -> Result<Vec<CommandRow>> {
        let conn = self.conn()?;
        let limit = limit.clamp(1, 500) as i64;
        let failure = if only_failed {
            "AND c.exit_code <> 0"
        } else {
            ""
        };

        let fts = fts_query(query);
        if fts.is_empty() {
            let sql = format!(
                "SELECT c.id, c.cmd, c.cwd, c.exit_code, c.output_truncated, c.sensitive,
                        c.duration_ms, c.ts
                 FROM commands c
                 WHERE 1 = 1 {failure}
                 ORDER BY c.ts DESC
                 LIMIT ?1"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![limit], map_row)?;
            return Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }

        let sql = format!(
            "SELECT c.id, c.cmd, c.cwd, c.exit_code, c.output_truncated, c.sensitive,
                    c.duration_ms, c.ts
             FROM commands c
             JOIN commands_fts f ON f.rowid = c.id
             WHERE commands_fts MATCH ?1 {failure}
             ORDER BY f.rank
             LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![fts, limit], map_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Dernières commandes, de la plus ancienne à la plus récente.
    pub fn recent(&self, limit: usize, only_failed: bool) -> Result<Vec<CommandDetail>> {
        let conn = self.conn()?;
        let limit = limit.clamp(1, 100) as i64;
        let failure = if only_failed { "WHERE exit_code <> 0" } else { "" };
        let sql = format!(
            "SELECT id, cmd, cwd, exit_code, output_truncated, sensitive,
                    duration_ms, ts, output
             FROM (
               SELECT id, cmd, cwd, exit_code, output_truncated, sensitive,
                      duration_ms, ts, output
               FROM commands {failure}
               ORDER BY ts DESC
               LIMIT ?1
             )
             ORDER BY ts ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(CommandDetail {
                row: CommandRow {
                    id: row.get(0)?,
                    cmd: row.get(1)?,
                    cwd: row.get(2)?,
                    exit_code: row.get(3)?,
                    output_truncated: row.get(4)?,
                    sensitive: row.get(5)?,
                    duration_ms: row.get(6)?,
                    ts: row.get(7)?,
                },
                output: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<CommandDetail>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, cmd, cwd, exit_code, output_truncated, sensitive,
                    duration_ms, ts, output
             FROM commands WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(CommandDetail {
            row: CommandRow {
                id: row.get(0)?,
                cmd: row.get(1)?,
                cwd: row.get(2)?,
                exit_code: row.get(3)?,
                output_truncated: row.get(4)?,
                sensitive: row.get(5)?,
                duration_ms: row.get(6)?,
                ts: row.get(7)?,
            },
            output: row.get(8)?,
        }))
    }

    pub fn list_providers(&self) -> Result<Vec<ProviderConfig>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT id, name, base_url, model FROM providers ORDER BY created_ts")?;
        let rows = stmt.query_map([], |row| {
            Ok(ProviderConfig {
                id: row.get(0)?,
                name: row.get(1)?,
                base_url: row.get(2)?,
                model: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_provider(&self, id: &str) -> Result<Option<ProviderConfig>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT id, name, base_url, model FROM providers WHERE id = ?1")?;
        let mut rows = stmt.query(params![id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(ProviderConfig {
            id: row.get(0)?,
            name: row.get(1)?,
            base_url: row.get(2)?,
            model: row.get(3)?,
        }))
    }

    pub fn save_provider(&self, provider: &ProviderConfig, ts: i64) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO providers (id, name, base_url, model, created_ts)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               base_url = excluded.base_url,
               model = excluded.model",
            params![provider.id, provider.name, provider.base_url, provider.model, ts],
        )?;
        Ok(())
    }

    pub fn delete_provider(&self, id: &str) -> Result<()> {
        self.conn()?
            .execute("DELETE FROM providers WHERE id = ?1", params![id])?;
        Ok(())
    }

    #[cfg(test)]
    pub fn count(&self) -> Result<i64> {
        Ok(self
            .conn()?
            .query_row("SELECT COUNT(*) FROM commands", [], |row| row.get(0))?)
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow!("connexion SQLite verrouillée"))
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// Transforme une saisie libre en requête FTS5 sûre : chaque mot devient un
/// préfixe, la ponctuation de l'utilisateur ne peut pas casser la syntaxe.
fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>()
        })
        .filter(|token| !token.is_empty())
        .map(|token| format!("{token}*"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CommandRow> {
    Ok(CommandRow {
        id: row.get(0)?,
        cmd: row.get(1)?,
        cwd: row.get(2)?,
        exit_code: row.get(3)?,
        output_truncated: row.get(4)?,
        sensitive: row.get(5)?,
        duration_ms: row.get(6)?,
        ts: row.get(7)?,
    })
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(cmd: &str, output: &str, exit_code: i32) -> CommandRecord {
        CommandRecord {
            cmd: cmd.into(),
            cwd: Some("/home/moi".into()),
            exit_code,
            output: output.into(),
            truncated: false,
            sensitive: false,
            duration_ms: 42,
        }
    }

    fn db() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.ensure_session("pty-1", "zsh", Some("/home/moi"), 1).unwrap();
        db
    }

    #[test]
    fn migration_idempotente() {
        let db = db();
        db.insert_command("pty-1", &record("ls", "fichier", 0), 10).unwrap();
        assert_eq!(db.count().unwrap(), 1);
    }

    #[test]
    fn recherche_fts_sur_commande_et_sortie() {
        let db = db();
        db.insert_command("pty-1", &record("tar -xzvf archive.tgz", "extraction ok", 0), 10)
            .unwrap();
        db.insert_command("pty-1", &record("ls", "archive.tgz", 0), 20)
            .unwrap();

        let by_cmd = db.search("tar xzvf", 10, false).unwrap();
        assert_eq!(by_cmd.len(), 1);
        assert_eq!(by_cmd[0].cmd, "tar -xzvf archive.tgz");

        let by_output = db.search("extraction", 10, false).unwrap();
        assert_eq!(by_output.len(), 1);
    }

    #[test]
    fn saisie_ponctuee_ne_casse_pas_fts() {
        let db = db();
        db.insert_command("pty-1", &record("echo \"it's fine\"", "ok", 0), 10).unwrap();
        assert!(db.search("\"it's (fine) *", 10, false).is_ok());
        assert!(db.search("!!!", 10, false).is_ok());
    }

    #[test]
    fn filtre_echecs_et_ordre_chronologique() {
        let db = db();
        db.insert_command("pty-1", &record("true", "", 0), 10).unwrap();
        db.insert_command("pty-1", &record("false", "", 1), 20).unwrap();

        let failed = db.search("", 10, true).unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].cmd, "false");

        let all = db.search("", 10, false).unwrap();
        assert_eq!(all[0].cmd, "false");
    }

    #[test]
    fn detail_contient_la_sortie() {
        let db = db();
        let id = db
            .insert_command("pty-1", &record("ls", "fichier1\nfichier2", 0), 10)
            .unwrap();
        let detail = db.get(id).unwrap().unwrap();
        assert_eq!(detail.output, "fichier1\nfichier2");
        assert_eq!(detail.row.cmd, "ls");
    }

    #[test]
    fn flags_sensible_et_tronque_persistes() {
        let db = db();
        let mut rec = record("sudo cat /etc/shadow", "", 0);
        rec.sensitive = true;
        rec.truncated = true;
        let id = db.insert_command("pty-1", &rec, 10).unwrap();
        let detail = db.get(id).unwrap().unwrap();
        assert!(detail.row.sensitive);
        assert!(detail.row.output_truncated);
    }

    #[test]
    fn providers_persistes_et_mis_a_jour() {
        let db = db();
        let provider = ProviderConfig {
            id: "ollama".into(),
            name: "Ollama".into(),
            base_url: "http://localhost:11434/v1".into(),
            model: "qwen2.5-coder:7b".into(),
        };
        db.save_provider(&provider, 1).unwrap();
        assert_eq!(db.list_providers().unwrap().len(), 1);

        let updated = ProviderConfig {
            model: "autre".into(),
            ..provider
        };
        db.save_provider(&updated, 2).unwrap();
        let stored = db.get_provider("ollama").unwrap().unwrap();
        assert_eq!(stored.model, "autre");
        assert_eq!(db.list_providers().unwrap().len(), 1);

        db.delete_provider("ollama").unwrap();
        assert!(db.get_provider("ollama").unwrap().is_none());
    }

    #[test]
    fn serialisation_attendue_par_le_frontend() {
        let db = db();
        let id = db
            .insert_command("pty-1", &record("ls", "sortie", 0), 10)
            .unwrap();
        let detail = db.get(id).unwrap().unwrap();
        let json = serde_json::to_value(&detail).unwrap();

        for key in ["id", "cmd", "cwd", "exitCode", "durationMs", "ts", "output"] {
            assert!(json.get(key).is_some(), "clé manquante : {key}");
        }
    }
}

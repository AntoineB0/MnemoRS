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
    include_str!("docs.sql"),
    include_str!("docs_sync.sql"),
    include_str!("fixes.sql"),
];

pub mod docs;
pub mod fixes;

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

/// Restreint un fichier ou un dossier à son propriétaire. L'historique contient
/// toutes les commandes exécutées et leurs sorties.
pub fn restrict(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("restriction des droits de {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("ouverture de {}", path.display()))?;
        // Les fichiers -wal et -shm héritent de l'umask : le dossier 0700 les protège.
        restrict(path, 0o600)?;
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

    pub fn replace_source(
        &self,
        source: &str,
        tool: Option<&str>,
        chunks: &[docs::NewChunk],
        embed_model: &str,
        ts: i64,
    ) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM doc_chunks WHERE source = ?1", params![source])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO doc_chunks
                   (source, tool, section, content, embed_model, embedding, ts)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for chunk in chunks {
                stmt.execute(params![
                    source,
                    tool,
                    chunk.section,
                    chunk.content,
                    embed_model,
                    docs::encode_embedding(&chunk.embedding),
                    ts,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn search_docs_fts(&self, query: &str, embed_model: &str, limit: usize) -> Result<Vec<i64>> {
        let fts = fts_query(query);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT c.id
             FROM doc_chunks c
             JOIN doc_chunks_fts f ON f.rowid = c.id
             WHERE doc_chunks_fts MATCH ?1 AND c.embed_model = ?2
             ORDER BY f.rank
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![fts, embed_model, limit as i64], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<i64>>>()?)
    }

    pub fn all_embeddings(&self, embed_model: &str) -> Result<Vec<(i64, Vec<f32>)>> {
        let conn = self.conn()?;
        let mut stmt =
            conn.prepare("SELECT id, embedding FROM doc_chunks WHERE embed_model = ?1")?;
        let rows = stmt.query_map(params![embed_model], |row| {
            let id: i64 = row.get(0)?;
            let bytes: Vec<u8> = row.get(1)?;
            Ok((id, docs::decode_embedding(&bytes)))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn doc_chunk(&self, id: i64) -> Result<Option<docs::DocChunk>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, source, tool, section, content, deleted FROM doc_chunks WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(docs::DocChunk {
            id: row.get(0)?,
            source: row.get(1)?,
            tool: row.get(2)?,
            section: row.get(3)?,
            content: row.get(4)?,
            deleted: row.get(5)?,
        }))
    }

    /// État d'une source : empreinte de contenu et statut.
    pub fn source_state(&self, source: &str) -> Result<Option<(String, String)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT hash, status FROM doc_sources WHERE source = ?1")?;
        let mut rows = stmt.query(params![source])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some((row.get(0)?, row.get(1)?)))
    }

    pub fn upsert_source(
        &self,
        source: &str,
        kind: &str,
        hash: &str,
        chunks: usize,
        status: &str,
        ts: i64,
    ) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO doc_sources (source, kind, hash, chunks, status, updated_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(source) DO UPDATE SET
               kind = excluded.kind,
               hash = excluded.hash,
               chunks = excluded.chunks,
               status = excluded.status,
               updated_ts = excluded.updated_ts",
            params![source, kind, hash, chunks as i64, status, ts],
        )?;
        Ok(())
    }

    pub fn mark_source_deleted(&self, source: &str) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE doc_chunks SET deleted = 1 WHERE source = ?1",
            params![source],
        )?;
        tx.execute(
            "UPDATE doc_sources SET status = 'deleted' WHERE source = ?1",
            params![source],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn restore_source(&self, source: &str) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE doc_chunks SET deleted = 0 WHERE source = ?1",
            params![source],
        )?;
        tx.execute(
            "UPDATE doc_sources SET status = 'active' WHERE source = ?1",
            params![source],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn sources_by_kind(&self, kind: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT source, status FROM doc_sources WHERE kind = ?1")?;
        let rows = stmt.query_map(params![kind], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn docs_stats(&self) -> Result<docs::DocsStats> {
        let conn = self.conn()?;
        let chunks = conn.query_row("SELECT COUNT(*) FROM doc_chunks", [], |row| row.get(0))?;
        let deleted =
            conn.query_row("SELECT COUNT(*) FROM doc_chunks WHERE deleted = 1", [], |row| {
                row.get(0)
            })?;
        let mut stmt = conn.prepare("SELECT DISTINCT source FROM doc_chunks ORDER BY source")?;
        let sources = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        let mut stmt = conn.prepare("SELECT DISTINCT embed_model FROM doc_chunks")?;
        let models = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(docs::DocsStats {
            chunks,
            deleted,
            sources,
            models,
        })
    }

    pub fn clear_docs(&self) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM doc_chunks", [])?;
        tx.execute("DELETE FROM doc_sources", [])?;
        tx.commit()?;
        Ok(())
    }

    pub fn tool_names(&self) -> Result<Vec<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT cmd FROM commands")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut names = std::collections::BTreeSet::new();
        for cmd in rows {
            let Some(word) = cmd?.split_whitespace().next().map(str::to_string) else {
                continue;
            };
            let name = word.rsplit('/').next().unwrap_or(&word).to_string();
            let valid = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'));
            if valid {
                names.insert(name);
            }
        }
        Ok(names.into_iter().collect())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn insert_fix(
        &self,
        failed_cmd_id: i64,
        fix_cmd_id: i64,
        confidence: f32,
        status: &str,
        origin: &str,
        ts: i64,
    ) -> Result<()> {
        self.conn()?.execute(
            "INSERT OR IGNORE INTO fixes
               (failed_cmd_id, fix_cmd_id, confidence, status, origin, created_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![failed_cmd_id, fix_cmd_id, confidence, status, origin, ts],
        )?;
        Ok(())
    }

    pub fn fix_for_command(&self, cmd_id: i64) -> Result<Option<fixes::FixRow>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT f.id, f.failed_cmd_id, f.fix_cmd_id, cf.cmd, cx.cmd, cx.cwd,
                    f.confidence, f.status, f.created_ts
             FROM fixes f
             JOIN commands cf ON cf.id = f.failed_cmd_id
             JOIN commands cx ON cx.id = f.fix_cmd_id
             WHERE f.failed_cmd_id = ?1 AND f.status <> 'rejected'
             ORDER BY f.confidence DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![cmd_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(fixes::FixRow {
            id: row.get(0)?,
            failed_cmd_id: row.get(1)?,
            fix_cmd_id: row.get(2)?,
            failed_cmd: row.get(3)?,
            fix_cmd: row.get(4)?,
            cwd: row.get(5)?,
            confidence: row.get(6)?,
            status: row.get(7)?,
            ts: row.get(8)?,
        }))
    }

    pub fn pending_fixes(&self, limit: usize) -> Result<Vec<fixes::FixRow>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT f.id, f.failed_cmd_id, f.fix_cmd_id, cf.cmd, cx.cmd, cx.cwd,
                    f.confidence, f.status, f.created_ts
             FROM fixes f
             JOIN commands cf ON cf.id = f.failed_cmd_id
             JOIN commands cx ON cx.id = f.fix_cmd_id
             WHERE f.status = 'suggested'
             ORDER BY f.confidence DESC, f.created_ts DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], map_fix)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_fix_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn()?.execute(
            "UPDATE fixes SET status = ?2 WHERE id = ?1",
            params![id, status],
        )?;
        Ok(())
    }

    pub fn delete_fix(&self, id: i64) -> Result<()> {
        self.conn()?
            .execute("DELETE FROM fixes WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Corrections confirmées dont la commande échouée ressemble à `cmd`.
    pub fn similar_fixes(
        &self,
        cmd: &str,
        exclude_failed_id: i64,
        limit: usize,
    ) -> Result<Vec<fixes::FixExample>> {
        let query = fts_or_query(cmd);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT cf.cmd, cx.cmd, cx.cwd
             FROM fixes f
             JOIN commands cf ON cf.id = f.failed_cmd_id
             JOIN commands cx ON cx.id = f.fix_cmd_id
             WHERE f.status IN ('auto', 'confirmed')
               AND f.failed_cmd_id <> ?2
               AND f.failed_cmd_id IN (
                 SELECT rowid FROM commands_fts WHERE commands_fts MATCH ?1
               )
             ORDER BY f.confidence DESC, f.created_ts DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![query, exclude_failed_id, limit as i64], |row| {
            Ok(fixes::FixExample {
                failed_cmd: row.get(0)?,
                fix_cmd: row.get(1)?,
                cwd: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn backfill_commands(&self, limit: usize) -> Result<Vec<fixes::BackfillCommand>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, cmd, cwd, exit_code, ts FROM (
               SELECT id, session_id, cmd, cwd, exit_code, ts
               FROM commands
               ORDER BY ts DESC
               LIMIT ?1
             )
             ORDER BY ts ASC",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(fixes::BackfillCommand {
                id: row.get(0)?,
                session_id: row.get(1)?,
                cmd: row.get(2)?,
                cwd: row.get(3)?,
                exit_code: row.get(4)?,
                ts: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
/// Les tokens sont recoupés sur la ponctuation pour que `archive.tgz` retrouve
/// bien `archive` et `tgz`, tels que FTS5 les indexe.
fn fts_query(input: &str) -> String {
    fts_tokens(input)
        .map(|token| format!("{token}*"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn fts_tokens(input: &str) -> impl Iterator<Item = &str> {
    input
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|token| token.chars().count() >= 2)
}

fn map_fix(row: &rusqlite::Row<'_>) -> rusqlite::Result<fixes::FixRow> {
    Ok(fixes::FixRow {
        id: row.get(0)?,
        failed_cmd_id: row.get(1)?,
        fix_cmd_id: row.get(2)?,
        failed_cmd: row.get(3)?,
        fix_cmd: row.get(4)?,
        cwd: row.get(5)?,
        confidence: row.get(6)?,
        status: row.get(7)?,
        ts: row.get(8)?,
    })
}

/// Requête FTS en OR : on veut les commandes proches, pas identiques.
fn fts_or_query(input: &str) -> String {
    fts_tokens(input)
        .map(|token| format!("{token}*"))
        .collect::<Vec<_>>()
        .join(" OR ")
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

    #[cfg(unix)]
    #[test]
    fn base_lisible_par_le_proprietaire_seulement() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("mnemors-perms-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mnemors.db");

        let _db = Db::open(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;

        assert_eq!(mode, 0o600);
        std::fs::remove_dir_all(&dir).ok();
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
    fn recherche_par_nom_de_fichier_ponctue() {
        let db = db();
        db.insert_command(
            "pty-1",
            &record("tar -xJf Ressources-20261006.zip", "", 2),
            10,
        )
        .unwrap();

        assert_eq!(db.search("Ressources-20261006.zip", 10, false).unwrap().len(), 1);
        assert_eq!(db.search("archive.tgz", 10, false).unwrap().len(), 0);
        assert_eq!(db.search("20261006", 10, false).unwrap().len(), 1);
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

    fn chunk(section: &str, content: &str, seed: f32) -> docs::NewChunk {
        docs::NewChunk {
            section: Some(section.into()),
            content: content.into(),
            embedding: vec![seed, 1.0, 0.0],
        }
    }

    #[test]
    fn remplacement_de_source_et_recherche_fts() {
        let db = db();
        db.replace_source(
            "man:tar",
            Some("tar"),
            &[
                chunk("OPTIONS", "extraire une archive avec -x", 0.1),
                chunk("OPTIONS", "compresser avec -c", 0.2),
            ],
            "bge-m3:1024",
            10,
        )
        .unwrap();

        let ids = db.search_docs_fts("extraire", "bge-m3:1024", 5).unwrap();
        assert_eq!(ids.len(), 1);
        let found = db.doc_chunk(ids[0]).unwrap().unwrap();
        assert!(found.content.contains("extraire"));
        assert_eq!(found.source, "man:tar");

        db.replace_source("man:tar", Some("tar"), &[chunk("NOM", "tar archive", 0.3)], "bge-m3:1024", 20)
            .unwrap();
        assert_eq!(db.docs_stats().unwrap().chunks, 1);
    }

    #[test]
    fn recherche_filtre_par_modele() {
        let db = db();
        db.replace_source("man:tar", Some("tar"), &[chunk("OPTIONS", "extraire", 0.1)], "modele-a", 10)
            .unwrap();
        assert!(db.search_docs_fts("extraire", "modele-b", 5).unwrap().is_empty());
        assert_eq!(db.search_docs_fts("extraire", "modele-a", 5).unwrap().len(), 1);
    }

    #[test]
    fn embeddings_relus_et_effaces() {
        let db = db();
        db.replace_source("man:tar", Some("tar"), &[chunk("OPTIONS", "texte", 0.5)], "m", 10)
            .unwrap();
        let all = db.all_embeddings("m").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1, vec![0.5, 1.0, 0.0]);

        db.clear_docs().unwrap();
        assert!(db.all_embeddings("m").unwrap().is_empty());
        assert_eq!(db.docs_stats().unwrap().chunks, 0);
    }

    #[test]
    fn reglages_persistes() {
        let db = db();
        assert!(db.get_setting("embed_model").unwrap().is_none());
        db.set_setting("embed_model", "bge-m3").unwrap();
        db.set_setting("embed_model", "autre").unwrap();
        assert_eq!(db.get_setting("embed_model").unwrap().as_deref(), Some("autre"));
    }

    #[test]
    fn outils_extraits_de_l_historique() {
        let db = db();
        db.insert_command("pty-1", &record("tar -xzf a.tgz", "", 0), 10).unwrap();
        db.insert_command("pty-1", &record("/usr/bin/git status", "", 0), 20).unwrap();
        db.insert_command("pty-1", &record("tar -tzf a.tgz", "", 0), 30).unwrap();
        assert_eq!(db.tool_names().unwrap(), vec!["git".to_string(), "tar".to_string()]);
    }

    #[test]
    fn corrections_similaires_retrouvees() {
        let db = db();
        let failed = db
            .insert_command("pty-1", &record("tar -xzf a.tgz", "", 2), 10)
            .unwrap();
        let fixed = db
            .insert_command("pty-1", &record("tar -xJf a.tar.xz", "", 0), 20)
            .unwrap();
        db.insert_fix(failed, fixed, 0.99, "auto", "live", 20).unwrap();

        let found = db.similar_fixes("tar -xzf b.tgz", 999, 5).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].fix_cmd, "tar -xJf a.tar.xz");

        assert!(db
            .similar_fixes("tar -xzf b.tgz", failed, 5)
            .unwrap()
            .is_empty());

        let fix = db.fix_for_command(failed).unwrap().unwrap();
        assert_eq!(fix.status, "auto");
        db.set_fix_status(fix.id, "rejected").unwrap();
        assert!(db
            .similar_fixes("tar -xzf b.tgz", 999, 5)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn suggestions_en_attente_isolees() {
        let db = db();
        let failed = db
            .insert_command("pty-1", &record("python app.py", "", 1), 10)
            .unwrap();
        let fixed = db
            .insert_command("pty-1", &record("pip install requests", "", 0), 20)
            .unwrap();
        db.insert_fix(failed, fixed, 0.7, "suggested", "live", 20)
            .unwrap();

        assert_eq!(db.pending_fixes(10).unwrap().len(), 1);
        assert!(db.similar_fixes("python app.py", 999, 5).unwrap().is_empty());
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

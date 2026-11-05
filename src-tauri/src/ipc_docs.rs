use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State};

use crate::docs::{self, Embedder, SyncReport};
use crate::storage::{docs as storage_docs, Db};
use crate::AppState;

const DEFAULT_EMBED_URL: &str = "http://localhost:11434/v1";
const DEFAULT_EMBED_MODEL: &str = "bge-m3";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocsSettings {
    pub base_url: String,
    pub model: String,
    pub auto_index: bool,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum IngestEvent {
    Progress { done: usize, total: usize, message: String },
    Done { report: SyncReport },
}

#[tauri::command]
pub fn docs_settings(state: State<'_, AppState>) -> Result<DocsSettings, String> {
    Ok(read_settings(&state.db))
}

#[tauri::command]
pub fn docs_set_settings(
    state: State<'_, AppState>,
    base_url: String,
    model: String,
    auto_index: bool,
) -> Result<(), String> {
    let db = &state.db;
    db.set_setting("embed_base_url", base_url.trim())
        .map_err(|e| format!("{e:#}"))?;
    db.set_setting("embed_model", model.trim())
        .map_err(|e| format!("{e:#}"))?;
    db.set_setting("auto_index", if auto_index { "1" } else { "0" })
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn docs_stats(state: State<'_, AppState>) -> Result<storage_docs::DocsStats, String> {
    state.db.docs_stats().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn docs_clear(state: State<'_, AppState>) -> Result<(), String> {
    state.db.clear_docs().map_err(|e| format!("{e:#}"))
}

/// `mode = "all"` inclut les outils (`man`/`--help`), `mode = "files"`
/// se limite aux fichiers du dossier courant.
#[tauri::command]
pub async fn docs_ingest(
    state: State<'_, AppState>,
    mode: Option<String>,
    on_event: Channel<IngestEvent>,
) -> Result<(), String> {
    let db = Arc::clone(&state.db);
    let settings = read_settings(&db);
    let cwd = current_cwd(&state);
    let include_tools = mode.as_deref().unwrap_or("all") == "all";

    let _guard = match state.ingest_running.swap(true, Ordering::Relaxed) {
        true => return Err("une indexation est déjà en cours".into()),
        false => RunningGuard(Arc::clone(&state.ingest_running)),
    };

    let sources = collect_sources(&db, &cwd, include_tools).await?;
    let embedder = Embedder::new(&settings.base_url, &settings.model);
    let cancel = Arc::clone(&state.ingest_cancel);
    cancel.store(false, Ordering::Relaxed);

    let report = docs::sync(
        &db,
        &embedder,
        &settings.model,
        &sources,
        Some(Path::new(&cwd)),
        &cancel,
        |done, total, message| {
            let _ = on_event.send(IngestEvent::Progress {
                done,
                total,
                message: message.to_string(),
            });
        },
    )
    .await
    .map_err(|error| format!("{error:#}"))?;

    if include_tools && !report.cancelled {
        store_system_fingerprint(&db);
    }

    let _ = on_event.send(IngestEvent::Done {
        report: report.clone(),
    });
    Ok(())
}

#[tauri::command]
pub fn docs_cancel(state: State<'_, AppState>) -> Result<(), String> {
    state.ingest_cancel.store(true, Ordering::Relaxed);
    Ok(())
}

/// Passe automatique : fichiers du dossier courant, plus les outils si la
/// base de paquets a changé. Ignorée si les embeddings ne sont pas locaux.
pub async fn auto_sync(app: AppHandle, db: Arc<Db>, cwd: String) -> anyhow::Result<SyncReport> {
    let settings = read_settings(&db);
    if !settings.auto_index || !is_local(&settings.base_url) {
        return Ok(SyncReport::default());
    }

    let tools_changed = system_changed(&db);
    let sources = collect_sources(&db, &cwd, tools_changed)
        .await
        .map_err(anyhow::Error::msg)?;
    let embedder = Embedder::new(&settings.base_url, &settings.model);
    let cancel = std::sync::atomic::AtomicBool::new(false);

    let report = docs::sync(
        &db,
        &embedder,
        &settings.model,
        &sources,
        Some(Path::new(&cwd)),
        &cancel,
        |_, _, _| {},
    )
    .await?;

    if tools_changed {
        store_system_fingerprint(&db);
    }
    let _ = app.emit("docs:indexed", &report);
    Ok(report)
}

async fn collect_sources(
    db: &Db,
    cwd: &str,
    include_tools: bool,
) -> Result<Vec<docs::SourceDoc>, String> {
    let tools = if include_tools {
        db.tool_names().map_err(|e| format!("{e:#}"))?
    } else {
        Vec::new()
    };
    let dir = cwd.to_string();

    tauri::async_runtime::spawn_blocking(move || {
        let mut sources = docs::tools(&tools);
        sources.extend(docs::readmes(Path::new(&dir)));
        sources
    })
    .await
    .map_err(|error| format!("{error:?}"))
}

fn system_changed(db: &Db) -> bool {
    match (
        docs::system::fingerprint(),
        db.get_setting("system_db_fingerprint").ok().flatten(),
    ) {
        (Some(current), Some(stored)) => current != stored,
        (Some(_), None) => true,
        _ => false,
    }
}

fn store_system_fingerprint(db: &Db) {
    if let Some(fingerprint) = docs::system::fingerprint() {
        let _ = db.set_setting("system_db_fingerprint", &fingerprint);
    }
}

pub fn current_cwd(state: &AppState) -> String {
    state
        .cwd
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        })
}

/// Vrai si l'URL pointe vers la machine locale. La comparaison porte sur l'hôte
/// exact : `localhost.exemple.com` n'est pas local.
fn is_local(url: &str) -> bool {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"));
    let Some(rest) = rest else { return false };

    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };

    matches!(host.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1")
}

pub fn read_settings(db: &Db) -> DocsSettings {
    DocsSettings {
        base_url: db
            .get_setting("embed_base_url")
            .ok()
            .flatten()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_EMBED_URL.to_string()),
        model: db
            .get_setting("embed_model")
            .ok()
            .flatten()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_EMBED_MODEL.to_string()),
        auto_index: db
            .get_setting("auto_index")
            .ok()
            .flatten()
            .map(|value| value != "0")
            .unwrap_or(true),
    }
}

struct RunningGuard(Arc<std::sync::atomic::AtomicBool>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::is_local;

    #[test]
    fn hotes_locaux_reconnus() {
        for url in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:11434",
            "http://[::1]:11434/v1",
            "https://localhost",
        ] {
            assert!(is_local(url), "{url} devrait être local");
        }
    }

    #[test]
    fn hotes_distants_refuses() {
        for url in [
            "http://localhost.exemple.com/v1",
            "http://127.0.0.1.exemple.com",
            "http://192.168.1.10:11434",
            "http://exemple.com/localhost",
            "ftp://localhost",
            "localhost:11434",
        ] {
            assert!(!is_local(url), "{url} ne devrait pas être local");
        }
    }
}

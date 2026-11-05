use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::State;

use crate::docs::{self, Embedder};
use crate::storage::{docs as storage_docs, now_ms, Db};
use crate::AppState;

const DEFAULT_EMBED_URL: &str = "http://localhost:11434/v1";
const DEFAULT_EMBED_MODEL: &str = "bge-m3";
const CHUNK_CHARS: usize = 1_200;
const EMBED_BATCH: usize = 32;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocsSettings {
    pub base_url: String,
    pub model: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum IngestEvent {
    Progress { done: usize, total: usize, message: String },
    Done { chunks: usize, sources: usize },
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
) -> Result<(), String> {
    let db = &state.db;
    db.set_setting("embed_base_url", base_url.trim())
        .map_err(|e| format!("{e:#}"))?;
    db.set_setting("embed_model", model.trim())
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

#[tauri::command]
pub async fn docs_ingest(
    state: State<'_, AppState>,
    on_event: Channel<IngestEvent>,
) -> Result<(), String> {
    let db = Arc::clone(&state.db);
    let settings = read_settings(&db);
    let cwd = state
        .cwd
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        });

    let tools = db.tool_names().map_err(|e| format!("{e:#}"))?;
    let sources = tauri::async_runtime::spawn_blocking(move || {
        docs::collect(&tools, Path::new(&cwd))
    })
    .await
    .map_err(|error| format!("{error:?}"))?;

    let total = sources.len();
    if total == 0 {
        let _ = on_event.send(IngestEvent::Done {
            chunks: 0,
            sources: 0,
        });
        return Ok(());
    }

    let embedder = Embedder::new(&settings.base_url, &settings.model);
    let mut chunks_total = 0;

    for (index, source) in sources.iter().enumerate() {
        let pieces = docs::chunk(&source.text, CHUNK_CHARS);
        let mut new_chunks = Vec::with_capacity(pieces.len());

        for batch in pieces.chunks(EMBED_BATCH) {
            let texts: Vec<String> = batch
                .iter()
                .map(|piece| match &piece.section {
                    Some(section) => format!("{section}\n{}", piece.content),
                    None => piece.content.clone(),
                })
                .collect();
            let vectors = embedder
                .embed(&texts)
                .await
                .map_err(|error| format!("{error:#}"))?;
            for (piece, vector) in batch.iter().zip(vectors) {
                new_chunks.push(storage_docs::NewChunk {
                    section: piece.section.clone(),
                    content: piece.content.clone(),
                    embedding: vector,
                });
            }
        }

        let dims = new_chunks.first().map(|chunk| chunk.embedding.len()).unwrap_or(0);
        if dims == 0 {
            continue;
        }
        let key = docs::model_key(&settings.model, dims);
        db.replace_source(
            &source.source,
            source.tool.as_deref(),
            &new_chunks,
            &key,
            now_ms(),
        )
        .map_err(|error| format!("{error:#}"))?;

        chunks_total += new_chunks.len();
        let _ = on_event.send(IngestEvent::Progress {
            done: index + 1,
            total,
            message: source.source.clone(),
        });
    }

    let _ = on_event.send(IngestEvent::Done {
        chunks: chunks_total,
        sources: total,
    });
    Ok(())
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
    }
}

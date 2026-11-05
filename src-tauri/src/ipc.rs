use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State};

use crate::capture::{Capture, CommandRecord};
use crate::fixes;
use crate::pty::{self, PtyEvent};
use crate::{storage, AppState};

/// Relie la sortie du PTY au parseur OSC 133 puis à SQLite.
struct Tap {
    capture: Capture,
    db: Arc<storage::Db>,
    session_id: String,
    app: AppHandle,
    cwd: Arc<Mutex<Option<String>>>,
    ingest_running: Arc<AtomicBool>,
    cwd_generation: Arc<AtomicU64>,
    pending_failure: Option<fixes::PendingFix>,
}

impl Tap {
    fn new(
        db: Arc<storage::Db>,
        session_id: String,
        app: AppHandle,
        cwd: Arc<Mutex<Option<String>>>,
        ingest_running: Arc<AtomicBool>,
        cwd_generation: Arc<AtomicU64>,
    ) -> Self {
        Self {
            capture: Capture::default(),
            db,
            session_id,
            app,
            cwd,
            ingest_running,
            cwd_generation,
            pending_failure: None,
        }
    }

    /// Repère qu'une commande réussie corrige l'échec précédent. Une commande
    /// de bruit ne consomme pas l'échec en attente : la vraie correction suit.
    fn track_fix(&mut self, id: i64, record: &CommandRecord) {
        let settings = fixes::settings(&self.db);
        let ts = storage::now_ms();

        if record.exit_code != 0 {
            if !fixes::is_noise(&record.cmd) {
                self.pending_failure = Some(fixes::PendingFix {
                    id,
                    cmd: record.cmd.clone(),
                    cwd: record.cwd.clone(),
                    ts,
                });
            }
            return;
        }

        let Some(pending) = self.pending_failure.take() else {
            return;
        };
        match fixes::evaluate(
            &pending,
            &record.cmd,
            record.cwd.as_deref(),
            record.exit_code,
            ts,
            &settings,
        ) {
            fixes::Decision::Keep => self.pending_failure = Some(pending),
            fixes::Decision::Fix { confidence, status } => {
                if let Err(error) = self
                    .db
                    .insert_fix(pending.id, id, confidence, status, "live", ts)
                {
                    log::warn!("correction non enregistrée : {error:#}");
                }
            }
            fixes::Decision::Drop => {}
        }
    }

    /// Réindexe en tâche de fond après un changement de dossier. La génération
    /// évite qu'un `cd` rapide lance plusieurs passes, le verrou évite le
    /// chevauchement avec une indexation manuelle.
    fn schedule_auto_sync(&self, cwd: String) {
        let generation = self.cwd_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let app = self.app.clone();
        let db = Arc::clone(&self.db);
        let running = Arc::clone(&self.ingest_running);
        let current = Arc::clone(&self.cwd_generation);

        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if current.load(Ordering::Relaxed) != generation {
                return;
            }
            if running.swap(true, Ordering::Relaxed) {
                return;
            }
            let _ = app.emit("docs:indexing", true);
            match crate::ipc_docs::auto_sync(app.clone(), db, cwd).await {
                Ok(report) if report.indexed > 0 || report.deleted > 0 => log::info!(
                    "indexation auto : {} réindexés, {} supprimés, {} chunks",
                    report.indexed,
                    report.deleted,
                    report.chunks
                ),
                Ok(_) => {}
                Err(error) => log::warn!("indexation automatique : {error:#}"),
            }
            let _ = app.emit("docs:indexing", false);
            running.store(false, Ordering::Relaxed);
        });
    }

    fn process(&mut self, input: &str) -> String {
        let (visible, records) = self.capture.feed(input);
        if let Some(cwd) = self.capture.cwd() {
            let changed = self
                .cwd
                .lock()
                .map(|mut slot| {
                    let changed = slot.as_deref() != Some(cwd);
                    *slot = Some(cwd.to_string());
                    changed
                })
                .unwrap_or(false);
            if changed {
                self.schedule_auto_sync(cwd.to_string());
            }
        }
        for record in records {
            match self
                .db
                .insert_command(&self.session_id, &record, storage::now_ms())
            {
                Ok(id) => {
                    log::info!(
                        "commande #{id} : {} (code {})",
                        record.cmd,
                        record.exit_code
                    );
                    self.track_fix(id, &record);
                    let _ = self.app.emit("history:new", id);
                }
                Err(error) => log::error!("enregistrement impossible : {error:#}"),
            }
        }
        visible
    }
}

#[tauri::command]
pub fn pty_spawn(
    app: AppHandle,
    state: State<'_, AppState>,
    cols: u16,
    rows: u16,
    cwd: Option<String>,
    on_event: Channel<PtyEvent>,
) -> Result<String, String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let session_id = pty::next_session_id();

    state
        .db
        .ensure_session(&session_id, &shell, cwd.as_deref(), storage::now_ms())
        .map_err(|error| format!("{error:#}"))?;

    if let Ok(mut shell_slot) = state.shell.lock() {
        *shell_slot = shell.clone();
    }
    let mut tap = Tap::new(
        Arc::clone(&state.db),
        session_id.clone(),
        app,
        Arc::clone(&state.cwd),
        Arc::clone(&state.ingest_running),
        Arc::clone(&state.cwd_generation),
    );
    let emit = move |event| on_event.send(event).is_ok();
    let session = pty::spawn(
        &shell,
        cwd.as_deref(),
        cols,
        rows,
        &state.data_dir,
        move |input| tap.process(input),
        emit,
    )
    .map_err(|error| format!("impossible de lancer {shell}: {error:#}"))?;

    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "état des sessions verrouillé".to_string())?;
    sessions.insert(session_id.clone(), session);
    Ok(session_id)
}

#[tauri::command]
pub fn history_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
    only_failed: Option<bool>,
) -> Result<Vec<storage::CommandRow>, String> {
    state
        .db
        .search(&query, limit.unwrap_or(200), only_failed.unwrap_or(false))
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn history_get(
    state: State<'_, AppState>,
    id: i64,
) -> Result<Option<storage::CommandDetail>, String> {
    state.db.get(id).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn pty_write(
    state: State<'_, AppState>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    let sessions = state
        .sessions
        .lock()
        .map_err(|_| "état des sessions verrouillé".to_string())?;
    let session = sessions.get(&session_id).ok_or("session inconnue")?;
    session.write(data.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pty_resize(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let sessions = state
        .sessions
        .lock()
        .map_err(|_| "état des sessions verrouillé".to_string())?;
    let session = sessions.get(&session_id).ok_or("session inconnue")?;
    session.resize(cols, rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pty_kill(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "état des sessions verrouillé".to_string())?;
    if let Some(mut session) = sessions.remove(&session_id) {
        session.kill();
    }
    Ok(())
}

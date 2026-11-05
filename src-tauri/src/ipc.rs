use std::sync::Arc;

use tauri::ipc::Channel;
use tauri::State;

use crate::capture::Capture;
use crate::pty::{self, PtyEvent};
use crate::{storage, AppState};

/// Relie la sortie du PTY au parseur OSC 133 puis à SQLite.
struct Tap {
    capture: Capture,
    db: Arc<storage::Db>,
    session_id: String,
}

impl Tap {
    fn new(db: Arc<storage::Db>, session_id: String) -> Self {
        Self {
            capture: Capture::default(),
            db,
            session_id,
        }
    }

    fn process(&mut self, input: &str) -> String {
        let (visible, records) = self.capture.feed(input);
        for record in records {
            match self
                .db
                .insert_command(&self.session_id, &record, storage::now_ms())
            {
                Ok(id) => log::info!(
                    "commande #{id} : {} (code {})",
                    record.cmd,
                    record.exit_code
                ),
                Err(error) => log::error!("enregistrement impossible : {error:#}"),
            }
        }
        visible
    }
}

#[tauri::command]
pub fn pty_spawn(
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

    let mut tap = Tap::new(Arc::clone(&state.db), session_id.clone());
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

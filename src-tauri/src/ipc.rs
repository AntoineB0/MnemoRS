use tauri::ipc::Channel;
use tauri::State;

use crate::pty::{self, PtyEvent};
use crate::AppState;

#[tauri::command]
pub fn pty_spawn(
    state: State<'_, AppState>,
    cols: u16,
    rows: u16,
    cwd: Option<String>,
    on_event: Channel<PtyEvent>,
) -> Result<String, String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let emit = move |event| on_event.send(event).is_ok();
    let (id, session) = pty::spawn(&shell, cwd.as_deref(), cols, rows, emit)
        .map_err(|e| format!("impossible de lancer {shell}: {e:#}"))?;

    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| "état des sessions verrouillé".to_string())?;
    sessions.insert(id.clone(), session);
    Ok(id)
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

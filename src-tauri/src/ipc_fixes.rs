use tauri::State;

use crate::fixes::{self, FixSettings};
use crate::storage::fixes::FixRow;
use crate::AppState;

#[tauri::command]
pub fn fixes_settings(state: State<'_, AppState>) -> Result<FixSettings, String> {
    Ok(fixes::settings(&state.db))
}

#[tauri::command]
pub fn fixes_set_settings(
    state: State<'_, AppState>,
    window_minutes: u32,
    auto_threshold: f32,
    suggest_threshold: f32,
) -> Result<(), String> {
    let db = &state.db;
    let window = window_minutes.clamp(1, 1440);
    let high = auto_threshold.clamp(0.0, 1.0);
    let low = suggest_threshold.clamp(0.0, 1.0).min(high);

    db.set_setting("fix_window_minutes", &window.to_string())
        .map_err(|e| format!("{e:#}"))?;
    db.set_setting("fix_auto_threshold", &high.to_string())
        .map_err(|e| format!("{e:#}"))?;
    db.set_setting("fix_suggest_threshold", &low.to_string())
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn fixes_pending(state: State<'_, AppState>) -> Result<Vec<FixRow>, String> {
    state.db.pending_fixes(50).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn fixes_for(state: State<'_, AppState>, cmd_id: i64) -> Result<Option<FixRow>, String> {
    state.db.fix_for_command(cmd_id).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn fixes_set_status(
    state: State<'_, AppState>,
    id: i64,
    status: String,
) -> Result<(), String> {
    if !matches!(status.as_str(), "confirmed" | "rejected") {
        return Err("statut invalide".into());
    }
    state
        .db
        .set_fix_status(id, &status)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn fixes_delete(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.db.delete_fix(id).map_err(|e| format!("{e:#}"))
}

use serde::Serialize;

/// Couple échec → correction, avec les textes des deux commandes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FixRow {
    pub id: i64,
    pub failed_cmd_id: i64,
    pub fix_cmd_id: i64,
    pub failed_cmd: String,
    pub fix_cmd: String,
    pub cwd: Option<String>,
    pub confidence: f32,
    pub status: String,
    pub ts: i64,
}

/// Exemple injecté dans le contexte du modèle.
#[derive(Debug, Clone)]
pub struct FixExample {
    pub failed_cmd: String,
    pub fix_cmd: String,
    pub cwd: Option<String>,
}

/// Ligne minimale pour rejouer l'heuristique sur l'historique existant.
pub struct BackfillCommand {
    pub id: i64,
    pub session_id: Option<String>,
    pub cmd: String,
    pub cwd: Option<String>,
    pub exit_code: Option<i32>,
    pub ts: i64,
}

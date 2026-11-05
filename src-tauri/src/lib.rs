mod capture;
mod docs;
mod harness;
mod ipc;
mod ipc_chat;
mod ipc_docs;
mod providers;
mod pty;
mod shell;
mod storage;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

use tauri::Manager;

pub struct AppState {
    pub sessions: Mutex<HashMap<String, pty::Session>>,
    pub cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub cwd: Arc<Mutex<Option<String>>>,
    pub shell: Mutex<String>,
    pub ingest_running: Arc<AtomicBool>,
    pub cwd_generation: Arc<AtomicU64>,
    pub db: Arc<storage::Db>,
    pub data_dir: PathBuf,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            ipc::pty_spawn,
            ipc::pty_write,
            ipc::pty_resize,
            ipc::pty_kill,
            ipc::history_search,
            ipc::history_get,
            ipc_chat::providers_list,
            ipc_chat::providers_save,
            ipc_chat::providers_delete,
            ipc_chat::providers_test,
            ipc_chat::chat_send,
            ipc_chat::chat_cancel,
            ipc_docs::docs_settings,
            ipc_docs::docs_set_settings,
            ipc_docs::docs_stats,
            ipc_docs::docs_clear,
            ipc_docs::docs_ingest,
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db = storage::Db::open(&data_dir.join("mnemors.db"))
                .map_err(|error| format!("{error:#}"))?;

            if db.list_providers().map(|list| list.is_empty()).unwrap_or(false) {
                let provider = providers::default_provider();
                db.save_provider(&provider, storage::now_ms())
                    .map_err(|error| format!("{error:#}"))?;
            }

            app.manage(AppState {
                sessions: Mutex::new(HashMap::new()),
                cancels: Mutex::new(HashMap::new()),
                cwd: Arc::new(Mutex::new(None)),
                shell: Mutex::new(String::new()),
                ingest_running: Arc::new(AtomicBool::new(false)),
                cwd_generation: Arc::new(AtomicU64::new(0)),
                db: Arc::new(db),
                data_dir,
            });

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}

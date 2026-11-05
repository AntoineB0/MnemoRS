use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;
use tauri::ipc::Channel;
use tauri::State;

use crate::harness::{self, CommandContext, ContextInput};
use crate::providers::{self, ChatEvent, ChatMessage, ProviderConfig};
use crate::storage::CommandDetail;
use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    #[serde(flatten)]
    config: ProviderConfig,
    has_key: bool,
}

#[tauri::command]
pub fn providers_list(state: State<'_, AppState>) -> Result<Vec<ProviderView>, String> {
    let providers = state.db.list_providers().map_err(|e| format!("{e:#}"))?;
    Ok(providers
        .into_iter()
        .map(|config| ProviderView {
            has_key: providers::get_key(&config.id).ok().flatten().is_some(),
            config,
        })
        .collect())
}

#[tauri::command]
pub fn providers_save(
    state: State<'_, AppState>,
    provider: ProviderConfig,
    api_key: Option<String>,
) -> Result<(), String> {
    if provider.id.trim().is_empty() || provider.base_url.trim().is_empty() {
        return Err("identifiant et URL requis".into());
    }
    if let Some(key) = api_key.filter(|key| !key.trim().is_empty()) {
        providers::set_key(&provider.id, key.trim()).map_err(|e| format!("{e:#}"))?;
    }
    state
        .db
        .save_provider(&provider, crate::storage::now_ms())
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub fn providers_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.db.delete_provider(&id).map_err(|e| format!("{e:#}"))?;
    providers::delete_key(&id).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn providers_test(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let provider = state
        .db
        .get_provider(&id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or("provider inconnu")?;
    let key = providers::get_key(&id).map_err(|e| format!("{e:#}"))?;

    let response = request(&provider, key.as_deref(), json!({
        "model": provider.model,
        "messages": [{ "role": "user", "content": "ping" }],
        "max_tokens": 1,
        "temperature": 0.2,
        "stream": false,
    }))
    .await
    .map_err(|e| format!("{e:#}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("HTTP {status} : {}", body.trim()));
    }
    Ok(format!("{} répond ({})", provider.name, provider.model))
}

#[tauri::command]
pub async fn chat_send(
    state: State<'_, AppState>,
    request_id: String,
    provider_id: String,
    messages: Vec<ChatMessage>,
    on_event: Channel<ChatEvent>,
) -> Result<(), String> {
    let provider = state
        .db
        .get_provider(&provider_id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or("provider inconnu")?;
    let key = providers::get_key(&provider_id).map_err(|e| format!("{e:#}"))?;

    let recent = state.db.recent(12, false).map_err(|e| format!("{e:#}"))?;
    let failures = state.db.recent(5, true).map_err(|e| format!("{e:#}"))?;
    let cwd = state.cwd.lock().ok().and_then(|slot| slot.clone());
    let shell = state
        .shell
        .lock()
        .map(|slot| slot.clone())
        .unwrap_or_default();
    let context = harness::build(&ContextInput {
        cwd: cwd.as_deref(),
        shell: if shell.is_empty() { "inconnu" } else { &shell },
        recent: recent.into_iter().map(to_context).collect(),
        failures: failures.into_iter().map(to_context).collect(),
    });

    let mut messages = messages;
    messages.insert(
        0,
        ChatMessage {
            role: "system".into(),
            content: context,
        },
    );

    let cancel = Arc::new(AtomicBool::new(false));
    if let Ok(mut cancels) = state.cancels.lock() {
        cancels.insert(request_id.clone(), Arc::clone(&cancel));
    }

    let emit = |event: ChatEvent| {
        let _ = on_event.send(event);
    };
    let result = stream_chat(&provider, key.as_deref(), messages, &cancel, emit).await;

    if let Ok(mut cancels) = state.cancels.lock() {
        cancels.remove(&request_id);
    }

    let event = match result {
        _ if cancel.load(Ordering::Relaxed) => ChatEvent::Cancelled,
        Ok(()) => ChatEvent::Done,
        Err(error) => ChatEvent::Error {
            message: format!("{error:#}"),
        },
    };
    let _ = on_event.send(event);
    Ok(())
}

#[tauri::command]
pub fn chat_cancel(state: State<'_, AppState>, request_id: String) -> Result<(), String> {
    let cancels = state
        .cancels
        .lock()
        .map_err(|_| "état des requêtes verrouillé".to_string())?;
    if let Some(cancel) = cancels.get(&request_id) {
        cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

fn to_context(detail: CommandDetail) -> CommandContext {
    CommandContext {
        cmd: detail.row.cmd,
        cwd: detail.row.cwd,
        exit_code: detail.row.exit_code,
        output: detail.output,
    }
}

async fn stream_chat<F>(
    provider: &ProviderConfig,
    key: Option<&str>,
    messages: Vec<ChatMessage>,
    cancel: &AtomicBool,
    emit: F,
) -> Result<()>
where
    F: Fn(ChatEvent),
{
    let response = request(
        provider,
        key,
        json!({
            "model": provider.model,
            "messages": messages,
            "temperature": 0.2,
            "stream": true,
        }),
    )
    .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("HTTP {status} : {}", body.trim());
    }

    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();

    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        buffer.extend_from_slice(&chunk.context("lecture du flux")?);

        for payload in providers::sse::take_events(&mut buffer) {
            if payload.trim() == "[DONE]" {
                return Ok(());
            }
            let value: serde_json::Value = match serde_json::from_str(&payload) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if let Some(text) = value["choices"][0]["delta"]["content"].as_str() {
                if !text.is_empty() {
                    emit(ChatEvent::Delta {
                        text: text.to_string(),
                    });
                }
            }
        }
    }

    Ok(())
}

async fn request(
    provider: &ProviderConfig,
    key: Option<&str>,
    body: serde_json::Value,
) -> Result<reqwest::Response> {
    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    let mut request = reqwest::Client::new()
        .post(&url)
        .timeout(std::time::Duration::from_secs(300))
        .json(&body);
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    request.send().await.context("appel au provider")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn le_contexte_contient_l_erreur_reelle() {
        let context = context_de_test();
        assert!(context.contains("tar -xzf archive.tar.gz"));
        assert!(context.contains("code 2"));
        assert!(context.contains("No such file or directory"));
        assert!(context.contains("/tmp"));
    }

    fn context_de_test() -> String {
        use crate::capture::CommandRecord;
        let db = crate::storage::Db::open_in_memory().unwrap();
        db.ensure_session("s", "zsh", Some("/tmp"), 1).unwrap();
        db.insert_command(
            "s",
            &CommandRecord {
                cmd: "tar -xzf archive.tar.gz".into(),
                cwd: Some("/tmp".into()),
                exit_code: 2,
                output: "tar: archive.tar.gz: Cannot open: No such file or directory".into(),
                truncated: false,
                sensitive: false,
                duration_ms: 5,
            },
            10,
        )
        .unwrap();

        let recent = db.recent(12, false).unwrap();
        let failures = db.recent(5, true).unwrap();
        harness::build(&ContextInput {
            cwd: Some("/tmp"),
            shell: "zsh",
            recent: recent.into_iter().map(to_context).collect(),
            failures: failures.into_iter().map(to_context).collect(),
        })
    }

    #[tokio::test]
    #[ignore = "nécessite Ollama en local sur le port 11434"]
    async fn repond_sur_la_commande_echouee() {
        let messages = vec![
            ChatMessage {
                role: "system".into(),
                content: context_de_test(),
            },
            ChatMessage {
                role: "user".into(),
                content: "Pourquoi ma dernière commande a échoué ?".into(),
            },
        ];
        let cancel = AtomicBool::new(false);
        let text = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&text);

        stream_chat(&providers::default_provider(), None, messages, &cancel, move |event| {
            if let ChatEvent::Delta { text } = event {
                sink.lock().unwrap().push_str(&text);
            }
        })
        .await
        .expect("stream Ollama");

        let answer = text.lock().unwrap().clone();
        println!("réponse : {answer}");
        assert!(!answer.trim().is_empty(), "réponse vide");
    }

    #[tokio::test]
    #[ignore = "nécessite Ollama en local sur le port 11434"]
    async fn stream_reel_depuis_ollama() {
        let provider = providers::default_provider();
        let messages = vec![ChatMessage {
            role: "user".into(),
            content: "Réponds uniquement par: bonjour".into(),
        }];
        let cancel = AtomicBool::new(false);
        let text = Arc::new(Mutex::new(String::new()));
        let sink = Arc::clone(&text);

        stream_chat(&provider, None, messages, &cancel, move |event| {
            if let ChatEvent::Delta { text } = event {
                sink.lock().unwrap().push_str(&text);
            }
        })
        .await
        .expect("stream Ollama");

        let text = text.lock().unwrap();
        assert!(!text.trim().is_empty(), "réponse vide");
        println!("réponse : {text}");
    }
}

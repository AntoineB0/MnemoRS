use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;
use tauri::ipc::Channel;
use tauri::State;

use crate::docs::{self, Embedder};
use crate::harness::{self, CommandContext, ContextInput, DocPassage, FixPassage};
use crate::providers::{self, ChatEvent, ChatMessage, ProviderConfig};
use crate::storage::CommandDetail;
use crate::AppState;

const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);
/// L'historique de conversation est borné : le contexte du modèle est petit,
/// et un long échange ne doit pas évincer les instructions système.
const HISTORY_MESSAGES: usize = 10;
const HISTORY_CHARS: usize = 4_000;

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

    let question = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.content.clone())
        .unwrap_or_default();
    let doc_passages = retrieve_docs(&state, &question).await;

    let recent = state.db.recent(12, false).map_err(|e| format!("{e:#}"))?;
    let failures = state.db.recent(5, true).map_err(|e| format!("{e:#}"))?;
    let fix_passages = similar_fixes(&state, &failures);
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
        docs: doc_passages,
        fixes: fix_passages,
    });

    let mut messages = trim_history(messages);
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
        Err(error) => {
            log::warn!("chat : {error:#}");
            ChatEvent::Error {
                message: format!("{error:#}"),
            }
        }
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

/// Corrections confirmées pour les échecs récents, dédupliquées par correction.
fn similar_fixes(state: &AppState, failures: &[CommandDetail]) -> Vec<FixPassage> {
    const MAX: usize = 3;
    let mut passages: Vec<FixPassage> = Vec::new();

    for failure in failures {
        if passages.len() >= MAX {
            break;
        }
        let Ok(examples) = state
            .db
            .similar_fixes(&failure.row.cmd, failure.row.id, 2)
        else {
            continue;
        };
        for example in examples {
            if passages.iter().any(|passage| passage.fix == example.fix_cmd) {
                continue;
            }
            passages.push(FixPassage {
                failed: example.failed_cmd,
                fix: example.fix_cmd,
                cwd: example.cwd,
            });
            if passages.len() >= MAX {
                break;
            }
        }
    }

    passages
}

/// Recherche documentaire ; toute panne est journalisée et n'empêche pas
/// la conversation de continuer sans documentation.
async fn retrieve_docs(state: &AppState, question: &str) -> Vec<DocPassage> {
    if question.trim().is_empty() {
        return Vec::new();
    }
    if state.db.docs_stats().map(|stats| stats.chunks).unwrap_or(0) == 0 {
        return Vec::new();
    }

    let settings = crate::ipc_docs::read_settings(&state.db);
    let embedder = Embedder::new(&settings.base_url, &settings.model);
    let vector = match embedder
        .embed_within(
            &[question.to_string()],
            std::time::Duration::from_secs(10),
        )
        .await
    {
        Ok(mut vectors) if !vectors.is_empty() => vectors.remove(0),
        Ok(_) => return Vec::new(),
        Err(error) => {
            log::warn!("embeddings indisponibles : {error:#}");
            return Vec::new();
        }
    };

    let key = docs::model_key(&settings.model, vector.len());
    match docs::search::search(&state.db, question, &vector, &key, 5) {
        Ok(chunks) => chunks
            .into_iter()
            .map(|chunk| DocPassage {
                source: chunk.source,
                section: chunk.section,
                content: chunk.content,
                deleted: chunk.deleted,
            })
            .collect(),
        Err(error) => {
            log::warn!("recherche documentaire : {error:#}");
            Vec::new()
        }
    }
}

/// Garde les derniers messages, dans l'ordre, sous un plafond de caractères.
/// Le dernier message est toujours conservé, tronqué si besoin : c'est la question.
fn trim_history(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut kept = Vec::new();
    let mut total = 0;

    for mut message in messages.into_iter().rev() {
        if kept.len() >= HISTORY_MESSAGES {
            break;
        }

        let remaining = HISTORY_CHARS.saturating_sub(total);
        if message.content.len() > remaining {
            if kept.is_empty() {
                message.content = truncate(&message.content, remaining);
                kept.push(message);
            }
            break;
        }

        total += message.content.len();
        kept.push(message);
    }

    kept.reverse();
    kept
}

/// Tronque sans couper un caractère multi-octets.
fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
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

    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        // Sans données pendant ce délai, on considère le provider muet : le
        // flux est coupé pour ne pas laisser l'interface bloquée.
        let next = tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await;
        let chunk = match next {
            Err(_) => bail!(
                "le modèle n'a rien envoyé pendant {} s",
                IDLE_TIMEOUT.as_secs()
            ),
            Ok(None) => break,
            Ok(Some(chunk)) => chunk.context("lecture du flux")?,
        };
        buffer.extend_from_slice(&chunk);

        for payload in providers::sse::take_events(&mut buffer) {
            if payload.trim() == "[DONE]" {
                return Ok(());
            }
            let value: serde_json::Value = match serde_json::from_str(&payload) {
                Ok(value) => value,
                Err(_) => continue,
            };

            if let Some(error) = value.get("error") {
                let message = error["message"]
                    .as_str()
                    .or_else(|| error.as_str())
                    .unwrap_or("erreur du provider");
                bail!("{message}");
            }

            if let Some(text) = value["choices"][0]["delta"]["content"].as_str() {
                if !text.is_empty() {
                    emit(ChatEvent::Delta {
                        text: text.to_string(),
                    });
                }
            }

            // Certains providers terminent sans envoyer [DONE].
            if value["choices"][0]["finish_reason"].is_string() {
                return Ok(());
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
    fn historique_borne_en_gardant_les_messages_recents() {
        let messages: Vec<ChatMessage> = (0..30)
            .map(|index| ChatMessage {
                role: if index % 2 == 0 { "user" } else { "assistant" }.into(),
                content: format!("message {index} {}", "x".repeat(300)),
            })
            .collect();

        let kept = trim_history(messages);
        assert!(kept.len() <= HISTORY_MESSAGES);
        assert!(kept.iter().map(|m| m.content.len()).sum::<usize>() <= HISTORY_CHARS);
        assert!(kept.last().unwrap().content.starts_with("message 29"));
    }

    #[test]
    fn historique_court_inchange() {
        let messages = vec![
            ChatMessage { role: "user".into(), content: "bonjour".into() },
            ChatMessage { role: "assistant".into(), content: "salut".into() },
        ];
        assert_eq!(trim_history(messages.clone()).len(), 2);
    }

    #[test]
    fn question_longue_conservee_tronquee() {
        let long = format!("début {}", "é".repeat(HISTORY_CHARS));
        let messages = vec![
            ChatMessage { role: "user".into(), content: "avant".into() },
            ChatMessage { role: "user".into(), content: long },
        ];

        let kept = trim_history(messages);

        assert_eq!(kept.len(), 1);
        assert!(kept[0].content.starts_with("début "));
        assert!(kept[0].content.len() <= HISTORY_CHARS);
    }

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
            docs: Vec::new(),
            fixes: Vec::new(),
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
    async fn erreur_du_provider_remontee() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf);
            let body = "data: {\"error\":{\"message\":\"model requires more system memory\"}}\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).unwrap();
        });

        let provider = ProviderConfig {
            id: "test".into(),
            name: "test".into(),
            base_url: format!("http://{addr}/v1"),
            model: "modele".into(),
        };
        let cancel = AtomicBool::new(false);
        let error = stream_chat(&provider, None, Vec::new(), &cancel, |_| {})
            .await
            .unwrap_err();

        assert!(
            format!("{error:#}").contains("memory"),
            "erreur : {error:#}"
        );
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

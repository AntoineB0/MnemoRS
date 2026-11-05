use serde::{Deserialize, Serialize};

/// Événements d'un tour de chat, poussés au frontend au fil du stream.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatEvent {
    Delta { text: String },
    Done,
    Cancelled,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

pub mod sse;

const KEYRING_SERVICE: &str = "mnemors";

pub fn set_key(provider_id: &str, key: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, provider_id)?;
    entry.set_password(key)?;
    Ok(())
}

pub fn get_key(provider_id: &str) -> anyhow::Result<Option<String>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, provider_id)?;
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn delete_key(provider_id: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, provider_id)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn default_provider() -> ProviderConfig {
    ProviderConfig {
        id: "ollama".into(),
        name: "Ollama (local)".into(),
        base_url: "http://localhost:11434/v1".into(),
        model: "qwen3.5:9b".into(),
    }
}

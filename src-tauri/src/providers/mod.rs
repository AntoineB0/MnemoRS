use serde::{Deserialize, Serialize};

/// Événements d'un tour de chat, poussés au frontend au fil du stream.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChatEvent {
    /// Étape en cours avant le premier token : recherche documentaire, puis
    /// attente du modèle.
    Phase { name: String },
    Delta { text: String },
    /// Compteurs exacts, quand le provider les renvoie en fin de stream.
    #[serde(rename_all = "camelCase")]
    Usage {
        prompt_tokens: u32,
        completion_tokens: u32,
    },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evenements_serialises_pour_le_frontend() {
        let usage = serde_json::to_value(ChatEvent::Usage {
            prompt_tokens: 13,
            completion_tokens: 796,
        })
        .unwrap();
        assert_eq!(usage["type"], "usage");
        assert_eq!(usage["promptTokens"], 13);
        assert_eq!(usage["completionTokens"], 796);

        let phase = serde_json::to_value(ChatEvent::Phase {
            name: "docs".into(),
        })
        .unwrap();
        assert_eq!(phase["type"], "phase");
        assert_eq!(phase["name"], "docs");
    }
}

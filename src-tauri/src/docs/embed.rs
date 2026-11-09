use anyhow::{bail, Context, Result};

/// Durée pendant laquelle Ollama garde le modèle d'embeddings chargé entre
/// deux requêtes. Sans cela, il le décharge après 5 min et le recharger prend
/// plus de 10 s quand le modèle de chat occupe déjà la VRAM.
const KEEP_ALIVE: &str = "30m";

/// Client d'embeddings OpenAI-compatible (Ollama par défaut).
pub struct Embedder {
    base_url: String,
    model: String,
}

impl Embedder {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
        }
    }

    /// Ollama expose `/api/embed`, qui accepte `keep_alive` ; son endpoint
    /// OpenAI l'ignore. On ne l'utilise que pour une instance locale.
    fn native_embed_url(&self) -> Option<String> {
        let base = self.base_url.strip_suffix("/v1").unwrap_or(&self.base_url);
        let local = ["http://localhost:11434", "http://127.0.0.1:11434"]
            .iter()
            .any(|prefix| base.starts_with(prefix));
        local.then(|| format!("{base}/api/embed"))
    }

    /// Envoie un lot de textes et renvoie les vecteurs dans l'ordre d'entrée.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.embed_within(texts, std::time::Duration::from_secs(120))
            .await
    }

    /// Variante avec délai court, pour ne pas bloquer une conversation quand
    /// le service d'embeddings est lent ou en cours de chargement.
    pub async fn embed_within(
        &self,
        texts: &[String],
        timeout: std::time::Duration,
    ) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let native = self.native_embed_url();
        let url = native
            .clone()
            .unwrap_or_else(|| format!("{}/embeddings", self.base_url));
        let payload = match native {
            Some(_) => serde_json::json!({
                "model": self.model,
                "input": texts,
                "keep_alive": KEEP_ALIVE,
            }),
            None => serde_json::json!({ "model": self.model, "input": texts }),
        };

        let response = reqwest::Client::new()
            .post(&url)
            .timeout(timeout)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("appel au service d'embeddings ({url})"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("HTTP {status} : {}", body.trim());
        }

        let body: serde_json::Value = response.json().await.context("réponse d'embeddings")?;

        if native.is_some() {
            let items = body["embeddings"]
                .as_array()
                .context("réponse d'embeddings sans champ embeddings")?;
            return items.iter().map(vector_from).collect();
        }

        let data = body["data"]
            .as_array()
            .context("réponse d'embeddings sans champ data")?;

        let mut vectors: Vec<(usize, Vec<f32>)> = Vec::with_capacity(data.len());
        for item in data {
            let index = item["index"].as_u64().unwrap_or(vectors.len() as u64) as usize;
            vectors.push((index, vector_from(&item["embedding"])?));
        }
        vectors.sort_by_key(|(index, _)| *index);
        Ok(vectors.into_iter().map(|(_, vector)| vector).collect())
    }
}

fn vector_from(value: &serde_json::Value) -> Result<Vec<f32>> {
    let items = value.as_array().context("vecteur absent")?;
    Ok(items
        .iter()
        .map(|value| value.as_f64().unwrap_or(0.0) as f32)
        .collect())
}

/// Clé stockée avec chaque chunk : un changement de modèle ou de dimension
/// rend les anciens vecteurs incompatibles.
pub fn model_key(model: &str, dims: usize) -> String {
    format!("{model}:{dims}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cle_de_modele_inclut_la_dimension() {
        assert_eq!(model_key("bge-m3", 1024), "bge-m3:1024");
        assert_ne!(model_key("bge-m3", 1024), model_key("bge-m3", 768));
    }

    #[test]
    fn endpoint_natif_reserve_a_ollama_local() {
        let local = Embedder::new("http://localhost:11434/v1", "bge-m3");
        assert_eq!(
            local.native_embed_url().as_deref(),
            Some("http://localhost:11434/api/embed")
        );

        let loopback = Embedder::new("http://127.0.0.1:11434/v1", "bge-m3");
        assert!(loopback.native_embed_url().is_some());

        let distant = Embedder::new("https://api.openai.com/v1", "text-embedding-3-small");
        assert_eq!(distant.native_embed_url(), None);
    }

    #[tokio::test]
    #[ignore = "nécessite Ollama et le modèle bge-m3"]
    async fn embeddings_reels_depuis_ollama() {
        let embedder = Embedder::new("http://localhost:11434/v1", "bge-m3");
        let vectors = embedder
            .embed(&["extraire une archive".into(), "lancer un service".into()])
            .await
            .expect("embeddings");
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].len(), 1024);
        assert_eq!(vectors[0].len(), vectors[1].len());
    }
}

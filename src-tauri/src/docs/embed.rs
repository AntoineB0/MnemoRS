use anyhow::{bail, Context, Result};

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

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Envoie un lot de textes et renvoie les vecteurs dans l'ordre d'entrée.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let url = format!("{}/embeddings", self.base_url);
        let response = reqwest::Client::new()
            .post(&url)
            .timeout(std::time::Duration::from_secs(120))
            .json(&serde_json::json!({ "model": self.model, "input": texts }))
            .send()
            .await
            .context("appel au service d'embeddings")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("HTTP {status} : {}", body.trim());
        }

        let body: serde_json::Value = response.json().await.context("réponse d'embeddings")?;
        let data = body["data"]
            .as_array()
            .context("réponse d'embeddings sans champ data")?;

        let mut vectors: Vec<(usize, Vec<f32>)> = Vec::with_capacity(data.len());
        for item in data {
            let index = item["index"].as_u64().unwrap_or(vectors.len() as u64) as usize;
            let vector = item["embedding"]
                .as_array()
                .context("vecteur absent")?
                .iter()
                .map(|value| value.as_f64().unwrap_or(0.0) as f32)
                .collect();
            vectors.push((index, vector));
        }
        vectors.sort_by_key(|(index, _)| *index);
        Ok(vectors.into_iter().map(|(_, vector)| vector).collect())
    }
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

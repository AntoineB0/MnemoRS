use serde::Serialize;

/// Chunk de documentation tel que stocké et recherché.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocChunk {
    pub id: i64,
    pub source: String,
    pub tool: Option<String>,
    pub section: Option<String>,
    pub content: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocsStats {
    pub chunks: i64,
    pub deleted: i64,
    pub sources: Vec<String>,
    pub models: Vec<String>,
}

/// Chunk prêt à être inséré : texte et vecteur déjà calculés.
pub struct NewChunk {
    pub section: Option<String>,
    pub content: String,
    pub embedding: Vec<f32>,
}

pub fn encode_embedding(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

pub fn decode_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Similarité cosinus ; 0 si l'un des vecteurs est nul ou de taille différente.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Reciprocal Rank Fusion : fusionne deux classements sans normaliser les scores.
pub fn rrf(fts: &[i64], vectors: &[i64], limit: usize) -> Vec<i64> {
    const K: f32 = 60.0;
    let mut scores: Vec<(i64, f32)> = Vec::new();

    let mut add = |id: i64, rank: usize| {
        let score = 1.0 / (K + rank as f32);
        match scores.iter_mut().find(|(existing, _)| *existing == id) {
            Some((_, total)) => *total += score,
            None => scores.push((id, score)),
        }
    };

    for (rank, id) in fts.iter().enumerate() {
        add(*id, rank);
    }
    for (rank, id) in vectors.iter().enumerate() {
        add(*id, rank);
    }

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores.into_iter().take(limit).map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_aller_retour() {
        let values = vec![0.5, -1.25, 3.0];
        assert_eq!(decode_embedding(&encode_embedding(&values)), values);
    }

    #[test]
    fn cosinus_vecteurs_identiques() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosinus_orthogonaux_et_nuls() {
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn rrf_favorise_les_presents_dans_les_deux_listes() {
        let fts = vec![10, 20, 30];
        let vectors = vec![20, 40];
        let fused = rrf(&fts, &vectors, 3);
        assert_eq!(fused[0], 20);
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn rrf_dedup_et_limite() {
        let fused = rrf(&[1, 1, 2], &[1], 10);
        assert_eq!(fused, vec![1, 2]);
    }
}

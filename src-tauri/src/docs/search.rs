use anyhow::Result;

use crate::storage::{docs, Db};

const FTS_CANDIDATES: usize = 20;
const VECTOR_CANDIDATES: usize = 20;

/// Recherche hybride : FTS5 pour les termes exacts, cosinus pour le sens,
/// fusionnés par RRF puis relus en base dans l'ordre du classement.
pub fn search(
    db: &Db,
    query: &str,
    query_vector: &[f32],
    embed_model: &str,
    limit: usize,
) -> Result<Vec<docs::DocChunk>> {
    let fts_ids = db.search_docs_fts(query, embed_model, FTS_CANDIDATES)?;

    let mut scored: Vec<(i64, f32)> = db
        .all_embeddings(embed_model)?
        .iter()
        .map(|(id, vector)| (*id, docs::cosine(query_vector, vector)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let vector_ids: Vec<i64> = scored
        .into_iter()
        .take(VECTOR_CANDIDATES)
        .map(|(id, _)| id)
        .collect();

    let mut chunks = Vec::new();
    for id in docs::rrf(&fts_ids, &vector_ids, limit) {
        if let Some(chunk) = db.doc_chunk(id)? {
            chunks.push(chunk);
        }
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::docs::NewChunk;

    fn db_with_docs() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.replace_source(
            "man:tar",
            Some("tar"),
            &[NewChunk {
                section: Some("OPTIONS".into()),
                content: "extraire une archive avec l'option -x".into(),
                embedding: vec![1.0, 0.0, 0.0],
            }],
            "bge-m3:3",
            1,
        )
        .unwrap();
        db.replace_source(
            "man:systemctl",
            Some("systemctl"),
            &[NewChunk {
                section: Some("COMMANDES".into()),
                content: "redémarrer un service avec restart".into(),
                embedding: vec![0.0, 1.0, 0.0],
            }],
            "bge-m3:3",
            2,
        )
        .unwrap();
        db
    }

    #[test]
    fn trouve_par_mot_cle_meme_sans_vecteur_proche() {
        let db = db_with_docs();
        let chunks = search(&db, "extraire", &[0.0, 1.0, 0.0], "bge-m3:3", 3).unwrap();
        assert!(chunks.iter().any(|c| c.source == "man:tar"));
    }

    #[test]
    fn trouve_par_sens_meme_sans_mot_cle() {
        let db = db_with_docs();
        let chunks = search(&db, "décompresser", &[1.0, 0.0, 0.0], "bge-m3:3", 3).unwrap();
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].source, "man:tar");
    }

    #[test]
    fn un_document_present_dans_les_deux_listes_passe_devant() {
        let db = db_with_docs();
        let chunks = search(&db, "archive", &[1.0, 0.0, 0.0], "bge-m3:3", 3).unwrap();
        assert_eq!(chunks[0].source, "man:tar");
    }

    #[test]
    fn ignore_les_autres_modeles() {
        let db = db_with_docs();
        let chunks = search(&db, "extraire", &[1.0, 0.0, 0.0], "autre:3", 3).unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn limite_le_nombre_de_resultats() {
        let db = db_with_docs();
        let chunks = search(&db, "archive service", &[1.0, 1.0, 0.0], "bge-m3:3", 1).unwrap();
        assert_eq!(chunks.len(), 1);
    }
}

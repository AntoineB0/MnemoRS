pub mod chunk;
pub mod embed;
pub mod ingest;
pub mod search;

pub use chunk::{chunk, clean_text};
pub use embed::{model_key, Embedder};
pub use ingest::collect;

#[cfg(test)]
mod integration {
    use super::*;
    use crate::storage::{docs as storage_docs, Db};
    use std::path::Path;

    #[tokio::test]
    #[ignore = "nécessite Ollama, bge-m3 et la page man de tar"]
    async fn pipeline_complet_sur_tar() {
        let db = Db::open_in_memory().unwrap();
        let sources = collect(&["tar".to_string()], Path::new("/tmp"));
        assert!(!sources.is_empty(), "aucune source collectée");

        let embedder = Embedder::new("http://localhost:11434/v1", "bge-m3");
        let mut total = 0;

        for source in &sources {
            let pieces = chunk(&source.text, 1200);
            let texts: Vec<String> = pieces
                .iter()
                .map(|piece| match &piece.section {
                    Some(section) => format!("{section}\n{}", piece.content),
                    None => piece.content.clone(),
                })
                .collect();
            let vectors = embedder.embed(&texts).await.expect("embeddings");
            let new_chunks: Vec<storage_docs::NewChunk> = pieces
                .iter()
                .zip(vectors)
                .map(|(piece, vector)| storage_docs::NewChunk {
                    section: piece.section.clone(),
                    content: piece.content.clone(),
                    embedding: vector,
                })
                .collect();
            let key = model_key("bge-m3", new_chunks[0].embedding.len());
            db.replace_source(&source.source, source.tool.as_deref(), &new_chunks, &key, 1)
                .unwrap();
            total += new_chunks.len();
        }

        assert!(total > 5, "chunks : {total}");

        let question = "comment extraire une archive tar.gz";
        let vector = embedder
            .embed(&[question.to_string()])
            .await
            .expect("embedding question")
            .remove(0);
        let key = model_key("bge-m3", vector.len());
        let found = search::search(&db, question, &vector, &key, 5).unwrap();

        assert!(!found.is_empty(), "aucun passage retrouvé");
        println!("meilleur passage : {} § {:?}", found[0].source, found[0].section);
        println!("extrait : {}", &found[0].content.chars().take(200).collect::<String>());
    }
}

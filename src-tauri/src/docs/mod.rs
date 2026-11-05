pub mod chunk;
pub mod embed;
pub mod ingest;
pub mod search;
pub mod sync;
pub mod system;

pub use chunk::{chunk, clean_text};
pub use embed::{model_key, Embedder};
pub use ingest::{readmes, tools, SourceDoc};
pub use sync::{sync, SyncReport};

#[cfg(test)]
mod integration {
    use super::*;
    use crate::storage::Db;

    #[tokio::test]
    #[ignore = "nécessite Ollama, bge-m3 et la page man de tar"]
    async fn pipeline_complet_sur_tar() {
        let db = Db::open_in_memory().unwrap();
        let sources = tools(&["tar".to_string()]);
        assert!(!sources.is_empty(), "aucune source collectée");

        let embedder = Embedder::new("http://localhost:11434/v1", "bge-m3");
        let report = sync(&db, &embedder, "bge-m3", &sources, None, |_, _, _| {})
            .await
            .expect("synchronisation");

        assert!(report.chunks > 5, "chunks : {}", report.chunks);

        let question = "comment extraire une archive tar.gz";
        let vector = embedder
            .embed(&[question.to_string()])
            .await
            .expect("embedding question")
            .remove(0);
        let key = model_key("bge-m3", vector.len());
        let found = search::search(&db, question, &vector, &key, 5).unwrap();

        assert!(!found.is_empty(), "aucun passage retrouvé");
        println!(
            "meilleur passage : {} § {:?}",
            found[0].source, found[0].section
        );
        println!(
            "extrait : {}",
            &found[0].content.chars().take(200).collect::<String>()
        );
    }

    #[tokio::test]
    #[ignore = "nécessite Ollama et bge-m3"]
    async fn deuxieme_passe_ne_reindexe_rien() {
        let db = Db::open_in_memory().unwrap();
        let sources = tools(&["tar".to_string()]);
        let embedder = Embedder::new("http://localhost:11434/v1", "bge-m3");

        let first = sync(&db, &embedder, "bge-m3", &sources, None, |_, _, _| {})
            .await
            .expect("première passe");
        assert!(first.indexed > 0);

        let second = sync(&db, &embedder, "bge-m3", &sources, None, |_, _, _| {})
            .await
            .expect("seconde passe");
        assert_eq!(second.indexed, 0, "rien ne doit être réindexé");
        assert_eq!(second.skipped, first.indexed);
        assert_eq!(second.chunks, 0);
    }

    #[test]
    fn fichiers_absents_marques_puis_restaures() {
        use crate::storage::docs::NewChunk;

        let db = Db::open_in_memory().unwrap();
        let dir = std::env::temp_dir().join("mnemors-test-suppression");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("README.md");
        std::fs::write(&file, "# Titre\n\ncontenu").unwrap();

        let sources = readmes(&dir);
        assert_eq!(sources.len(), 1);

        // insertion directe pour éviter un appel réseau
        db.replace_source(
            &sources[0].source,
            None,
            &[NewChunk {
                section: Some("Titre".into()),
                content: "contenu".into(),
                embedding: vec![1.0, 0.0],
            }],
            "m:2",
            1,
        )
        .unwrap();
        db.upsert_source(&sources[0].source, "file", "hash", 1, "active", 1)
            .unwrap();

        std::fs::remove_file(&file).unwrap();
        let state = db.source_state(&sources[0].source).unwrap().unwrap();
        assert_eq!(state.1, "active");

        db.mark_source_deleted(&sources[0].source).unwrap();
        let chunk = db.doc_chunk(1).unwrap().unwrap();
        assert!(chunk.deleted);
        assert_eq!(db.docs_stats().unwrap().deleted, 1);

        db.restore_source(&sources[0].source).unwrap();
        assert!(!db.doc_chunk(1).unwrap().unwrap().deleted);

        std::fs::remove_dir_all(&dir).ok();
    }
}

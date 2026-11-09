use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;

use super::{chunk, model_key, Embedder, SourceDoc};
use crate::storage::{docs as storage_docs, now_ms, Db};

const CHUNK_CHARS: usize = 1_200;
const EMBED_BATCH: usize = 32;

#[derive(Debug, Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub indexed: usize,
    pub skipped: usize,
    pub restored: usize,
    pub deleted: usize,
    pub chunks: usize,
    pub failed: usize,
    pub cancelled: bool,
}

enum Outcome {
    Skipped,
    Restored,
    Indexed(usize),
    Cancelled,
}

/// Met l'index à jour : seules les sources dont l'empreinte de contenu a
/// changé repassent par les embeddings. Les fichiers disparus sous `scope`
/// sont marqués supprimés plutôt qu'effacés.
pub async fn sync<F>(
    db: &Db,
    embedder: &Embedder,
    model: &str,
    sources: &[SourceDoc],
    scope: Option<&Path>,
    cancel: &AtomicBool,
    progress: F,
) -> Result<SyncReport>
where
    F: Fn(usize, usize, &str),
{
    let mut report = SyncReport::default();
    let total = sources.len();

    for (index, source) in sources.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }

        match sync_one(db, embedder, model, source, cancel).await {
            Ok(Outcome::Skipped) => report.skipped += 1,
            Ok(Outcome::Restored) => report.restored += 1,
            Ok(Outcome::Indexed(chunks)) => {
                report.indexed += 1;
                report.chunks += chunks;
            }
            Ok(Outcome::Cancelled) => {
                report.cancelled = true;
                break;
            }
            Err(error) => {
                log::warn!("indexation de {} : {error:#}", source.source);
                report.failed += 1;
            }
        }
        progress(index + 1, total, &source.source);
    }

    if let Some(scope) = scope {
        let present: HashSet<&str> = sources.iter().map(|source| source.source.as_str()).collect();
        for (source, status) in db.sources_by_kind("file")? {
            if status != "active" || present.contains(&source.as_str()) {
                continue;
            }
            // Hors périmètre n'est pas « supprimé » : on ne marque que les
            // fichiers qui ont réellement disparu du disque.
            let path = Path::new(&source);
            if path.starts_with(scope) && !path.exists() {
                db.mark_source_deleted(&source)?;
                report.deleted += 1;
            }
        }
    }

    // Les remplacements de sources laissent des segments FTS5 qui ralentissent
    // les requêtes à préfixes ; on les fusionne une fois la passe terminée.
    if report.indexed > 0 || report.deleted > 0 {
        if let Err(error) = db.optimize_docs_fts() {
            log::warn!("optimisation de l'index FTS : {error:#}");
        }
    }

    Ok(report)
}

async fn sync_one(
    db: &Db,
    embedder: &Embedder,
    model: &str,
    source: &SourceDoc,
    cancel: &AtomicBool,
) -> Result<Outcome> {
    let hash = blake3::hash(source.text.as_bytes()).to_hex().to_string();

    if let Some((stored, status)) = db.source_state(&source.source)? {
        if stored == hash {
            if status == "deleted" {
                db.restore_source(&source.source)?;
                return Ok(Outcome::Restored);
            }
            return Ok(Outcome::Skipped);
        }
    }

    let pieces = chunk(&source.text, CHUNK_CHARS);
    let mut new_chunks = Vec::with_capacity(pieces.len());

    for batch in pieces.chunks(EMBED_BATCH) {
        if cancel.load(Ordering::Relaxed) {
            return Ok(Outcome::Cancelled);
        }
        let texts: Vec<String> = batch
            .iter()
            .map(|piece| match &piece.section {
                Some(section) => format!("{section}\n{}", piece.content),
                None => piece.content.clone(),
            })
            .collect();
        let vectors = embedder.embed(&texts).await?;
        for (piece, vector) in batch.iter().zip(vectors) {
            new_chunks.push(storage_docs::NewChunk {
                section: piece.section.clone(),
                content: piece.content.clone(),
                embedding: vector,
            });
        }
    }

    let Some(dims) = new_chunks.first().map(|chunk| chunk.embedding.len()) else {
        return Ok(Outcome::Skipped);
    };
    let key = model_key(model, dims);
    db.replace_source(
        &source.source,
        source.tool.as_deref(),
        &new_chunks,
        &key,
        now_ms(),
    )?;
    db.upsert_source(
        &source.source,
        source.kind.as_str(),
        &hash,
        new_chunks.len(),
        "active",
        now_ms(),
    )?;

    Ok(Outcome::Indexed(new_chunks.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docs::ingest::SourceKind;

    fn source(name: &str) -> SourceDoc {
        SourceDoc {
            source: name.into(),
            kind: SourceKind::Tool,
            tool: None,
            text: format!("documentation de {name}"),
        }
    }

    #[tokio::test]
    async fn une_source_en_echec_n_arrete_pas_l_indexation() {
        let db = Db::open_in_memory().unwrap();
        let embedder = Embedder::new("http://127.0.0.1:1/v1", "test");
        let sources = vec![source("un"), source("deux")];

        let report = sync(&db, &embedder, "test", &sources, None, &AtomicBool::new(false), |_, _, _| {})
            .await
            .expect("la synchronisation doit continuer malgré les échecs");

        assert_eq!(report.failed, 2);
        assert_eq!(report.indexed, 0);
    }

    #[tokio::test]
    async fn annulation_avant_le_premier_document() {
        let db = Db::open_in_memory().unwrap();
        let embedder = Embedder::new("http://127.0.0.1:1/v1", "test");
        let sources = vec![source("un"), source("deux")];
        let cancel = AtomicBool::new(true);

        let report = sync(&db, &embedder, "test", &sources, None, &cancel, |_, _, _| {})
            .await
            .expect("l'annulation n'est pas une erreur");

        assert!(report.cancelled);
        assert_eq!(report.indexed, 0);
        // Aucun appel d'embedding : le service injoignable n'a pas été sollicité.
        assert_eq!(report.failed, 0);
    }
}

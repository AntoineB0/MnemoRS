use std::path::Path;

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
    progress: F,
) -> Result<SyncReport>
where
    F: Fn(usize, usize, &str),
{
    let mut report = SyncReport::default();
    let total = sources.len();

    for (index, source) in sources.iter().enumerate() {
        let hash = blake3::hash(source.text.as_bytes()).to_hex().to_string();

        if let Some((stored, status)) = db.source_state(&source.source)? {
            if stored == hash {
                if status == "deleted" {
                    db.restore_source(&source.source)?;
                    report.restored += 1;
                } else {
                    report.skipped += 1;
                }
                progress(index + 1, total, &source.source);
                continue;
            }
        }

        let pieces = chunk(&source.text, CHUNK_CHARS);
        let mut new_chunks = Vec::with_capacity(pieces.len());

        for batch in pieces.chunks(EMBED_BATCH) {
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
            continue;
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

        report.indexed += 1;
        report.chunks += new_chunks.len();
        progress(index + 1, total, &source.source);
    }

    if let Some(scope) = scope {
        let present: Vec<&str> = sources.iter().map(|source| source.source.as_str()).collect();
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

    Ok(report)
}

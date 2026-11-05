pub mod chunk;
pub mod embed;
pub mod ingest;
pub mod search;

pub use chunk::{chunk, clean_text, Chunk};
pub use embed::{model_key, Embedder};
pub use ingest::{collect, SourceDoc};

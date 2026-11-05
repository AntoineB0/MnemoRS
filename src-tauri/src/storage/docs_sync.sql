ALTER TABLE doc_chunks ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;

CREATE TABLE doc_sources (
  source TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  hash TEXT NOT NULL,
  chunks INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  updated_ts INTEGER NOT NULL
);

CREATE INDEX doc_sources_kind ON doc_sources(kind, status);

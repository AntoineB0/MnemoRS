CREATE TABLE doc_chunks (
  id INTEGER PRIMARY KEY,
  source TEXT NOT NULL,
  tool TEXT,
  section TEXT,
  content TEXT NOT NULL,
  embed_model TEXT NOT NULL,
  embedding BLOB NOT NULL,
  ts INTEGER NOT NULL
);

CREATE INDEX doc_chunks_source ON doc_chunks(source);
CREATE INDEX doc_chunks_model ON doc_chunks(embed_model);

CREATE VIRTUAL TABLE doc_chunks_fts USING fts5(
  content,
  section,
  content='doc_chunks',
  content_rowid='id',
  tokenize='unicode61'
);

CREATE TRIGGER doc_chunks_ai AFTER INSERT ON doc_chunks BEGIN
  INSERT INTO doc_chunks_fts(rowid, content, section)
  VALUES (new.id, new.content, new.section);
END;

CREATE TRIGGER doc_chunks_ad AFTER DELETE ON doc_chunks BEGIN
  INSERT INTO doc_chunks_fts(doc_chunks_fts, rowid, content, section)
  VALUES ('delete', old.id, old.content, old.section);
END;

CREATE TRIGGER doc_chunks_au AFTER UPDATE ON doc_chunks BEGIN
  INSERT INTO doc_chunks_fts(doc_chunks_fts, rowid, content, section)
  VALUES ('delete', old.id, old.content, old.section);
  INSERT INTO doc_chunks_fts(rowid, content, section)
  VALUES (new.id, new.content, new.section);
END;

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

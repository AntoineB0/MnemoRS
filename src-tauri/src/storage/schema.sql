CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  shell TEXT NOT NULL,
  started_ts INTEGER NOT NULL,
  cwd_start TEXT
);

CREATE TABLE commands (
  id INTEGER PRIMARY KEY,
  session_id TEXT REFERENCES sessions(id),
  cmd TEXT NOT NULL,
  cwd TEXT,
  exit_code INTEGER,
  output TEXT,
  output_truncated INTEGER NOT NULL DEFAULT 0,
  sensitive INTEGER NOT NULL DEFAULT 0,
  duration_ms INTEGER,
  ts INTEGER NOT NULL
);

CREATE INDEX commands_ts ON commands(ts DESC);

CREATE VIRTUAL TABLE commands_fts USING fts5(
  cmd,
  output,
  content='commands',
  content_rowid='id',
  tokenize='unicode61'
);

CREATE TRIGGER commands_ai AFTER INSERT ON commands BEGIN
  INSERT INTO commands_fts(rowid, cmd, output)
  VALUES (new.id, new.cmd, new.output);
END;

CREATE TRIGGER commands_ad AFTER DELETE ON commands BEGIN
  INSERT INTO commands_fts(commands_fts, rowid, cmd, output)
  VALUES ('delete', old.id, old.cmd, old.output);
END;

CREATE TRIGGER commands_au AFTER UPDATE ON commands BEGIN
  INSERT INTO commands_fts(commands_fts, rowid, cmd, output)
  VALUES ('delete', old.id, old.cmd, old.output);
  INSERT INTO commands_fts(rowid, cmd, output)
  VALUES (new.id, new.cmd, new.output);
END;

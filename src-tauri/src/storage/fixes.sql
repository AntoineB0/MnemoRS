CREATE TABLE fixes (
  id INTEGER PRIMARY KEY,
  failed_cmd_id INTEGER NOT NULL REFERENCES commands(id) ON DELETE CASCADE,
  fix_cmd_id INTEGER NOT NULL REFERENCES commands(id) ON DELETE CASCADE,
  confidence REAL NOT NULL,
  status TEXT NOT NULL,
  origin TEXT NOT NULL,
  created_ts INTEGER NOT NULL,
  UNIQUE(failed_cmd_id, fix_cmd_id)
);

CREATE INDEX fixes_failed ON fixes(failed_cmd_id, status);
CREATE INDEX fixes_status ON fixes(status, created_ts DESC);

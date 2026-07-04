CREATE TABLE memory_index (
  workspace_id TEXT PRIMARY KEY REFERENCES workspace(id) ON DELETE CASCADE,
  model_id     TEXT NOT NULL,
  dimension    INTEGER NOT NULL,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);

CREATE TABLE memory_chunk (
  id           TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  source_kind  TEXT NOT NULL,
  source_id    TEXT,
  text         TEXT NOT NULL,
  embedding    BLOB NOT NULL,
  dimension    INTEGER NOT NULL,
  content_hash TEXT NOT NULL,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  UNIQUE (workspace_id, content_hash)
);

CREATE INDEX idx_memory_chunk_ws_created ON memory_chunk(workspace_id, created_at);

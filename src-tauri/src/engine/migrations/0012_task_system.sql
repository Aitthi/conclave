CREATE TABLE task (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  slug TEXT NOT NULL,
  title TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'planned'
    CHECK (state IN ('planned','claimed','in_progress','review','merged','abandoned')),
  owner_agent_id TEXT,
  implementer_agent_id TEXT,
  file_boundary TEXT NOT NULL DEFAULT '[]',
  design_canon TEXT,
  plan TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (workspace_id, slug)
);
CREATE TABLE task_event (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('note','state','gate','challenge','ruling')),
  actor_agent_id TEXT,
  payload TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);
CREATE INDEX idx_task_event_task ON task_event(task_id, created_at);
CREATE TABLE task_watch (
  task_id TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  PRIMARY KEY (task_id, agent_id)
);

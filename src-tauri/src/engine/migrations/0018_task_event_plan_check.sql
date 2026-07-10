-- lead-council v1 (plan edit group 4): admit the typed 'plan_check' event
-- kind on the append-only task ledger (spec 2026-07-10 "Plan Check" step 6).
-- SQLite cannot ALTER a CHECK constraint, so rebuild task_event in place —
-- same columns and index as 0012_task_system.sql, all rows preserved.
CREATE TABLE task_event_new (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL,
  kind TEXT NOT NULL
    CHECK (kind IN ('note','state','gate','challenge','ruling','plan_check')),
  actor_agent_id TEXT,
  payload TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);
INSERT INTO task_event_new
  SELECT id, task_id, kind, actor_agent_id, payload, created_at FROM task_event;
DROP TABLE task_event;
ALTER TABLE task_event_new RENAME TO task_event;
CREATE INDEX idx_task_event_task ON task_event(task_id, created_at);

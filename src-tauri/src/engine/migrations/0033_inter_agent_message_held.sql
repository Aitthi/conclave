-- Inject outbox (spec docs/superpowers/specs/2026-09-19-inject-outbox-coalescing-design.md):
-- a message can now sit in a per-target outbox before delivery, status 'held'.
-- SQLite cannot ALTER a CHECK constraint, so rebuild inter_agent_message in
-- place — same columns and BOTH indexes as 0001_init.sql:188-189, all rows
-- preserved
-- (same idiom as 0018_task_event_plan_check.sql).
CREATE TABLE inter_agent_message_new (
    id               TEXT PRIMARY KEY,
    from_instance_id TEXT NOT NULL REFERENCES workspace_agent(id),
    to_instance_id   TEXT NOT NULL REFERENCES workspace_agent(id),
    text             TEXT NOT NULL,
    status           TEXT NOT NULL CHECK(status IN ('queued', 'delivered', 'held')),
    auto_submitted   INTEGER,
    created_at       TEXT NOT NULL
);
INSERT INTO inter_agent_message_new
  SELECT id, from_instance_id, to_instance_id, text, status, auto_submitted, created_at
  FROM inter_agent_message;
DROP TABLE inter_agent_message;
ALTER TABLE inter_agent_message_new RENAME TO inter_agent_message;
CREATE INDEX idx_inter_agent_msg_to   ON inter_agent_message(to_instance_id);
CREATE INDEX idx_inter_agent_msg_from ON inter_agent_message(from_instance_id);

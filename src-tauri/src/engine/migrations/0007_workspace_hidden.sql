-- A hidden ephemeral workspace backs an agent-assisted skill-draft session
-- (see docs/specs/2026-07-02-skill-editor-agent-assist-design.md). It is a
-- completely normal `workspace` row otherwise — every existing spawn/session
-- code path works against it unmodified — it is simply excluded from
-- `workspace.list` so it never appears in the normal workspace switcher.
ALTER TABLE workspace ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;

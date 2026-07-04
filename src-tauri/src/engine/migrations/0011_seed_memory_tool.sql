-- Seed the `memory` core tool so agents can call remember/search/delete via
-- their existing tool channel (workspace memory system, migration 0009),
-- independently toggleable per agent via `agent_tool` (unlike `tool-conclave`,
-- which represents the whole cli.exec command surface as one row).
INSERT OR IGNORE INTO tool (id, name, kind, icon, is_core)
VALUES ('tool-memory', 'Memory', 'builtin', 'brain', 1);

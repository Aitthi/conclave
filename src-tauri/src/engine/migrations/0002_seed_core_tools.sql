-- Seed built-in, core (undeletable) tools. Keyed on a stable id so re-running
-- the migration is harmless. `conclave` exposes the app's own command surface
-- (the cli.exec / UDS router) as a core tool present on every agent.
INSERT OR IGNORE INTO tool (id, name, kind, icon, is_core)
VALUES ('tool-conclave', 'Conclave', 'builtin', 'terminal', 1);

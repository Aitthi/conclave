-- Builtin skills now come from a bundled `skills/` folder, never the DB (see
-- docs/adr/0002-builtin-skills-from-bundled-folder.md) — every remaining
-- `skill` row is, structurally, a user-authored custom skill. SQLite 3.35+
-- supports DROP COLUMN for a plain column with a self-referencing CHECK
-- constraint (kind's CHECK only referenced kind itself, not another column).
ALTER TABLE skill DROP COLUMN kind;

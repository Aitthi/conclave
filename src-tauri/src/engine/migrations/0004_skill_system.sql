-- Extend the dormant `skill` table (0001_init.sql) with a builtin/custom
-- discriminator (mirrors `tool.kind`) and the actual instructional content
-- previously missing — `description` stays a short UI blurb, `content` is
-- what gets injected into a launched cli agent's skill sidecar file.
ALTER TABLE skill ADD COLUMN kind TEXT NOT NULL DEFAULT 'custom' CHECK(kind IN ('builtin', 'custom'));
ALTER TABLE skill ADD COLUMN content TEXT NOT NULL DEFAULT '';

-- No builtin skill rows are seeded yet in v1 — the mechanism ships with zero
-- rows; product can add `INSERT OR IGNORE INTO skill (...) VALUES (...)` rows
-- in a later migration without needing to touch this one.

-- Snapshot of which skill ids were actually used at the last launch (JSON
-- array, ordered: builtin first, then custom by agent_skill.sort_order — see
-- repo::skill::content_for_agent). Compared against an agent definition's
-- CURRENT attachments to show a "Restart to apply" badge on drift.
ALTER TABLE session ADD COLUMN launched_skill_ids TEXT;

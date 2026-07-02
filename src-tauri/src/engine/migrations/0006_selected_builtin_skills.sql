-- Per-AgentDefinition selection of OPTIONAL builtin skills (`mandatory: false`
-- in their SKILL.md frontmatter — see ADR 0003). Mandatory builtins are
-- always included and need no persisted selection. Cannot reuse `agent_skill`
-- (its skill_id column is FK-enforced against the `skill` table, and builtin
-- ids are never DB rows — see ADR 0002) so this mirrors the existing
-- `session.launched_skill_ids` JSON-array-column pattern instead.
ALTER TABLE agent_definition ADD COLUMN selected_builtin_skill_ids TEXT;

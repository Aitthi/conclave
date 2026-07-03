-- Role system (ADR 0005). A role is a first-class bundle: display name, a
-- one-paragraph job description baked verbatim into the preamble + roster, and
-- a default skill set. Builtin roles ship as a bundled `roles/<id>/ROLE.md`
-- folder (never DB rows — mirrors ADR 0002's builtin-skills pattern), so this
-- table holds ONLY user-authored custom roles. `skill_ids` is a JSON array of
-- skill ids (builtin folder ids or custom `skill` row ids); it is deliberately
-- NOT FK-enforced (builtin skill ids are never DB rows) — unknown ids are
-- filtered at read time, mirroring `effective_builtin_skills`.
CREATE TABLE role (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    description TEXT NOT NULL,
    skill_ids   TEXT NOT NULL DEFAULT '[]',
    created_at  TEXT NOT NULL
);

-- An agent definition's chosen role: a builtin slug (e.g. 'lead') or a custom
-- `role.id`. NULL for agents created before the role system. The legacy
-- free-text `role` column (0001_init.sql) stays as a display fallback — new
-- saves write both (`role` = the role's display name). Deleting a custom role
-- NULLs this back-pointer (see repo::role::delete) while leaving `role` intact
-- so nothing loses its label. No FK: builtin role ids are never DB rows.
ALTER TABLE agent_definition ADD COLUMN role_id TEXT;

-- First-class Antigravity CLI support.
--
-- SQLite cannot amend the cli_kind CHECK in place, so rebuild the table while
-- preserving every v27 column. The migration runner executes this file on one
-- connection with foreign_keys disabled BEFORE BEGIN; otherwise dropping the
-- referenced table would cascade through the inbound relation tables.

CREATE TABLE agent_definition_v28 (
    id                         TEXT PRIMARY KEY,
    name                       TEXT NOT NULL,
    role                       TEXT,
    type                       TEXT NOT NULL CHECK(type IN ('cli', 'chat', 'orchestrator')),
    cli_kind                   TEXT CHECK(cli_kind IN ('claude-code', 'codex', 'antigravity', 'custom')),
    color                      TEXT,
    provider_id                TEXT REFERENCES provider(id),
    model                      TEXT,
    harness_mode               TEXT NOT NULL CHECK(harness_mode IN ('own', 'central')),
    share_blackboard           INTEGER,
    auto_submit_injected       INTEGER,
    allowed_senders            TEXT CHECK(allowed_senders IN ('all', 'selected', 'none')),
    created_at                 TEXT NOT NULL,
    permission_mode            TEXT,
    custom_args                TEXT,
    custom_env                 TEXT,
    secret_env_keys            TEXT,
    context_window             TEXT,
    selected_builtin_skill_ids TEXT,
    role_id                    TEXT,
    default_level              TEXT CHECK(default_level IN ('junior', 'mid', 'senior', 'principal')),
    rtk_enabled                INTEGER,
    proxy_enabled              INTEGER,
    effort                     TEXT CHECK(effort IN ('low', 'medium', 'high'))
);

INSERT INTO agent_definition_v28 (
    id, name, role, type, cli_kind, color, provider_id, model, harness_mode,
    share_blackboard, auto_submit_injected, allowed_senders, created_at,
    permission_mode, custom_args, custom_env, secret_env_keys, context_window,
    selected_builtin_skill_ids, role_id, default_level, rtk_enabled,
    proxy_enabled, effort
)
SELECT
    id, name, role, type, cli_kind, color, provider_id, model, harness_mode,
    share_blackboard, auto_submit_injected, allowed_senders, created_at,
    permission_mode, custom_args, custom_env, secret_env_keys, context_window,
    selected_builtin_skill_ids, role_id, default_level, rtk_enabled,
    proxy_enabled, NULL
FROM agent_definition;

DROP TABLE agent_definition;
ALTER TABLE agent_definition_v28 RENAME TO agent_definition;

-- Conclave — initial schema
-- All 19 entity tables. Column names are snake_case; serde/IPC mapping
-- layer handles camelCase translation in later milestones.
-- Run inside a BEGIN/COMMIT transaction from the migration runner.

CREATE TABLE workspace (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    folder_path TEXT NOT NULL,
    color       TEXT,
    created_at  TEXT NOT NULL
);

CREATE TABLE provider (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL CHECK(name IN ('anthropic', 'openai', 'local', 'custom')),
    base_url   TEXT,
    masked_key TEXT,
    stored_in  TEXT NOT NULL DEFAULT 'keychain' CHECK(stored_in IN ('keychain')),
    status     TEXT CHECK(status IN ('connected', 'offline'))
);

CREATE TABLE agent_definition (
    id                   TEXT PRIMARY KEY,
    name                 TEXT NOT NULL,
    role                 TEXT,
    type                 TEXT NOT NULL CHECK(type IN ('cli', 'chat', 'orchestrator')),
    cli_kind             TEXT CHECK(cli_kind IN ('claude-code', 'codex', 'custom')),
    color                TEXT,
    provider_id          TEXT REFERENCES provider(id),
    model                TEXT,
    harness_mode         TEXT NOT NULL CHECK(harness_mode IN ('own', 'central')),
    share_blackboard     INTEGER,
    auto_submit_injected INTEGER,
    allowed_senders      TEXT CHECK(allowed_senders IN ('all', 'selected', 'none')),
    created_at           TEXT NOT NULL
);

CREATE TABLE workspace_agent (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    agent_def_id TEXT NOT NULL REFERENCES agent_definition(id),
    status       TEXT NOT NULL CHECK(status IN ('running', 'idle', 'waiting')),
    added_at     TEXT NOT NULL,
    UNIQUE(workspace_id, agent_def_id)
);

CREATE TABLE session (
    id                 TEXT PRIMARY KEY,
    workspace_agent_id TEXT NOT NULL UNIQUE REFERENCES workspace_agent(id) ON DELETE CASCADE,
    context_tokens     INTEGER,
    context_limit      INTEGER,
    started_at         TEXT NOT NULL,
    last_active_at     TEXT
);

CREATE TABLE message (
    id               TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    role             TEXT NOT NULL CHECK(role IN ('user', 'agent', 'tool', 'system')),
    text             TEXT,
    from_instance_id TEXT REFERENCES workspace_agent(id),
    injected         INTEGER,
    auto_submitted   INTEGER,
    created_at       TEXT NOT NULL
);

CREATE TABLE inter_agent_message (
    id               TEXT PRIMARY KEY,
    from_instance_id TEXT NOT NULL REFERENCES workspace_agent(id),
    to_instance_id   TEXT NOT NULL REFERENCES workspace_agent(id),
    text             TEXT NOT NULL,
    status           TEXT NOT NULL CHECK(status IN ('queued', 'delivered')),
    auto_submitted   INTEGER,
    created_at       TEXT NOT NULL
);

CREATE TABLE blackboard_entry (
    id             TEXT PRIMARY KEY,
    workspace_id   TEXT NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    key            TEXT NOT NULL,
    value          TEXT,
    last_writer_id TEXT REFERENCES workspace_agent(id),
    updated_at     TEXT NOT NULL,
    UNIQUE(workspace_id, key)
);

CREATE TABLE blackboard_activity (
    id          TEXT PRIMARY KEY,
    entry_id    TEXT NOT NULL REFERENCES blackboard_entry(id) ON DELETE CASCADE,
    instance_id TEXT NOT NULL REFERENCES workspace_agent(id),
    action      TEXT NOT NULL CHECK(action IN ('read', 'write')),
    at          TEXT NOT NULL
);

CREATE TABLE snapshot (
    id               TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    type             TEXT NOT NULL CHECK(type IN ('auto', 'manual', 'handoff')),
    label            TEXT,
    summary          TEXT,
    tokens           INTEGER,
    trigger_pct      INTEGER,
    prev_snapshot_id TEXT REFERENCES snapshot(id),
    carried_forward  TEXT,
    diff             TEXT,
    created_at       TEXT NOT NULL
);

CREATE TABLE artifact (
    id         TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES message(id) ON DELETE CASCADE,
    filename   TEXT,
    html       TEXT,
    sandboxed  INTEGER,
    created_at TEXT NOT NULL
);

CREATE TABLE tool (
    id      TEXT PRIMARY KEY,
    name    TEXT NOT NULL,
    kind    TEXT NOT NULL CHECK(kind IN ('builtin', 'plugin', 'mcp')),
    icon    TEXT,
    is_core INTEGER
);

CREATE TABLE agent_tool (
    agent_def_id TEXT REFERENCES agent_definition(id) ON DELETE CASCADE,
    tool_id      TEXT REFERENCES tool(id) ON DELETE CASCADE,
    enabled      INTEGER,
    PRIMARY KEY(agent_def_id, tool_id)
);

CREATE TABLE skill (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT,
    icon        TEXT
);

-- `order` is a SQL reserved word; column is named `sort_order` instead.
CREATE TABLE agent_skill (
    agent_def_id TEXT REFERENCES agent_definition(id) ON DELETE CASCADE,
    skill_id     TEXT REFERENCES skill(id) ON DELETE CASCADE,
    sort_order   INTEGER,
    PRIMARY KEY(agent_def_id, skill_id)
);

CREATE TABLE fusion_config (
    id                TEXT PRIMARY KEY,
    orchestrator_id   TEXT NOT NULL UNIQUE REFERENCES workspace_agent(id) ON DELETE CASCADE,
    judge_model       TEXT,
    force_fusion      INTEGER,
    panel_temperature REAL,
    max_tool_calls    INTEGER
);

CREATE TABLE fusion_panel_member (
    fusion_config_id TEXT REFERENCES fusion_config(id) ON DELETE CASCADE,
    agent_def_id     TEXT REFERENCES agent_definition(id) ON DELETE CASCADE,
    sort_order       INTEGER,
    PRIMARY KEY(fusion_config_id, agent_def_id)
);

CREATE TABLE fusion_run (
    id             TEXT PRIMARY KEY,
    session_id     TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
    prompt         TEXT NOT NULL,
    judge_analysis TEXT,
    synthesized    TEXT,
    created_at     TEXT NOT NULL
);

CREATE TABLE fusion_panel_response (
    id            TEXT PRIMARY KEY,
    fusion_run_id TEXT NOT NULL REFERENCES fusion_run(id) ON DELETE CASCADE,
    instance_id   TEXT REFERENCES workspace_agent(id),
    answer        TEXT,
    status        TEXT NOT NULL CHECK(status IN ('running', 'done', 'error')),
    created_at    TEXT NOT NULL
);

-- Indexes on hot foreign keys
CREATE INDEX idx_message_session_created    ON message(session_id, created_at);
CREATE INDEX idx_workspace_agent_workspace  ON workspace_agent(workspace_id);
CREATE INDEX idx_snapshot_session           ON snapshot(session_id);
CREATE INDEX idx_blackboard_entry_workspace ON blackboard_entry(workspace_id);
CREATE INDEX idx_inter_agent_msg_to         ON inter_agent_message(to_instance_id);
CREATE INDEX idx_inter_agent_msg_from       ON inter_agent_message(from_instance_id);
CREATE INDEX idx_fusion_run_session         ON fusion_run(session_id);
CREATE INDEX idx_fusion_panel_resp_run      ON fusion_panel_response(fusion_run_id);

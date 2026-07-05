-- Position System (spec §2.1): add optional reporting-line metadata to
-- `workspace_agent`. Both fields are nullable so existing rows and the
-- current create/instantiate paths remain valid during rollout.
ALTER TABLE workspace_agent
    ADD COLUMN level TEXT
    CHECK (level IN ('junior', 'mid', 'senior', 'principal'));

ALTER TABLE workspace_agent
    ADD COLUMN supervisor_agent_id TEXT
    REFERENCES workspace_agent(id) ON DELETE SET NULL;

CREATE INDEX idx_workspace_agent_supervisor
    ON workspace_agent(supervisor_agent_id);

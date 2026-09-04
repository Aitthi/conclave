-- Persistent workspace and per-agent lifecycle controls.
-- Existing user workspaces intentionally upgrade stopped: merely opening the
-- app must never launch agents before an explicit Start. Individual agents
-- remain active unless the user explicitly stops one.

ALTER TABLE workspace
    ADD COLUMN run_state TEXT NOT NULL DEFAULT 'stopped'
    CHECK (run_state IN ('started', 'stopped'));

ALTER TABLE workspace_agent
    ADD COLUMN availability TEXT NOT NULL DEFAULT 'active'
    CHECK (availability IN ('active', 'stopped'));

-- Runtime registrations are process-local and empty after an upgrade/restart;
-- normalize the persisted transient mirror so stopped workspaces never render
-- a stale pre-upgrade `running`/`waiting` status.
UPDATE workspace_agent SET status = 'idle';

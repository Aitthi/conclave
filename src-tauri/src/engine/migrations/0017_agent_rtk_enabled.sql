-- rtk (Claude Code hook) toggle per agent definition. Nullable INTEGER:
-- NULL means "not set" and is treated as enabled (house style — new bool
-- columns default ON via NULL, see global constraints). Only an explicit 0
-- disables rtk for the agent.
ALTER TABLE agent_definition
    ADD COLUMN rtk_enabled INTEGER;

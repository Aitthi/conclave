-- Context-proxy opt-in per agent definition (agent-proxy spec D8, double
-- opt-in). Nullable INTEGER: NULL means "not set" and is treated as DISABLED —
-- a deliberate asymmetry with rtk_enabled (whose NULL defaults ON). Only an
-- explicit 1 routes the agent's spawn env through the loopback context proxy.
ALTER TABLE agent_definition
    ADD COLUMN proxy_enabled INTEGER;

-- Default level seed on the agent definition: remembered across remove/re-add,
-- but still copied onto the workspace instance only at instantiate time.
ALTER TABLE agent_definition
    ADD COLUMN default_level TEXT
    CHECK (default_level IN ('junior', 'mid', 'senior', 'principal'));

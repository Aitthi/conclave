-- M-CLI-config: per-agent Claude Code / CLI launch configuration.
--
-- All columns are nullable so existing rows migrate cleanly (SQLite ADD COLUMN
-- appends to the end of the table; FromRow / chain-builder bind by NAME, so
-- column order is irrelevant to the Rust side).
--
-- `custom_env` holds a JSON object of NON-secret env vars (e.g. ANTHROPIC_BASE_URL,
-- ANTHROPIC_MODEL). Secret-looking values (AUTH_TOKEN / API_KEY / *_SECRET …) are
-- NEVER stored here — they go to the macOS Keychain under account
-- `agent_env:{agent_id}:{KEY}`, and only their NAMES are recorded in
-- `secret_env_keys` (a JSON array of strings) so the spawn path knows which to
-- fetch back and merge.

ALTER TABLE agent_definition ADD COLUMN permission_mode TEXT;
ALTER TABLE agent_definition ADD COLUMN custom_args     TEXT;
ALTER TABLE agent_definition ADD COLUMN custom_env      TEXT;
ALTER TABLE agent_definition ADD COLUMN secret_env_keys TEXT;
ALTER TABLE agent_definition ADD COLUMN context_window  TEXT;

-- Durable measured AI usage: one event relation plus coverage and importer
-- cursor state (docs/plans/2026-09-05-usage-engine.md, contract
-- docs/plans/2026-09-05-usage-overview-contract.md).
--
-- Design notes that the column list alone does not carry:
--   * There is deliberately NO synthetic total_tokens column. A display total
--     is input+output and is computed only when BOTH are known.
--   * input_tokens already INCLUDES cached input; output_tokens already
--     INCLUDES reasoning. The cache_/reasoning_ columns are subsets kept for
--     provenance and must never be added back into a total.
--   * Missing usage is NULL, never 0. NULL means "not observed"; 0 means "the
--     source reported zero".
--   * workspace_id is NULLABLE on purpose: a Library draft run legitimately
--     has no workspace (commands/draft.rs DraftRequest.workspace_id is
--     Option). The contract forbids attributing those to the selected
--     workspace, so they are stored unscoped and surfaced under a wire-only
--     "__unscoped__" bucket (preflight correction b1140da9).

CREATE TABLE model_usage_event (
    id                      TEXT    PRIMARY KEY NOT NULL,
    -- Versioned, source-namespaced idempotency key, e.g.
    -- 'claude-code:v1:<sessionId>:<requestId>'. UNIQUE is what makes replay,
    -- restart and duplicate source rows a no-op instead of inflation.
    event_key               TEXT    NOT NULL UNIQUE,

    -- Scope captured at collection time. All nullable but workspace_id is the
    -- only one that is routinely absent (unscoped draft).
    workspace_id            TEXT,
    workspace_agent_id      TEXT,
    session_id              TEXT,
    generation              INTEGER,

    source_kind             TEXT    NOT NULL,
    source_version          TEXT    NOT NULL,
    event_kind              TEXT    NOT NULL
                                    CHECK (event_kind IN ('response', 'invocation')),

    -- Source-side identity, kept for reconciliation and diagnostics only.
    source_session_id       TEXT,
    source_request_id       TEXT,
    source_response_id      TEXT,

    -- occurred_at is the SOURCE's own timestamp (never filesystem mtime).
    occurred_at             TEXT    NOT NULL,
    recorded_at             TEXT    NOT NULL,

    provider                TEXT,
    requested_model         TEXT,
    served_model            TEXT,

    -- Every counter is bounded by 2^40 (1099511627776). Real responses are
    -- many orders of magnitude smaller; the ceiling exists so a grouped SUM can
    -- never overflow SQLite's 64-bit integer (that needs 2^22 rows at the max)
    -- and never gets promoted to REAL. repo::model_usage::insert_event turns an
    -- out-of-range counter into NULL (unknown) with diagnostic
    -- 'counter_out_of_range' BEFORE it reaches this CHECK; the CHECK is the
    -- backstop for any other writer.
    input_tokens            INTEGER CHECK (input_tokens  IS NULL OR input_tokens  BETWEEN 0 AND 1099511627776),
    output_tokens           INTEGER CHECK (output_tokens IS NULL OR output_tokens BETWEEN 0 AND 1099511627776),
    cache_read_input_tokens INTEGER CHECK (cache_read_input_tokens  IS NULL OR cache_read_input_tokens  BETWEEN 0 AND 1099511627776),
    cache_write_input_tokens INTEGER CHECK (cache_write_input_tokens IS NULL OR cache_write_input_tokens BETWEEN 0 AND 1099511627776),
    reasoning_output_tokens INTEGER CHECK (reasoning_output_tokens  IS NULL OR reasoning_output_tokens  BETWEEN 0 AND 1099511627776),

    -- 'known' only when input AND output are both observed; 'partial' when one
    -- side is known; 'unknown' when neither is.
    token_completeness      TEXT    NOT NULL
                                    CHECK (token_completeness IN ('known', 'partial', 'unknown')),
    -- A conflicting source group stays as ONE activity but contributes no
    -- measured tokens until reconciled evidence agrees.
    validity                TEXT    NOT NULL DEFAULT 'valid'
                                    CHECK (validity IN ('valid', 'conflict')),
    diagnostic_code         TEXT
);

-- Range scans are always time-ordered; the scope columns narrow them.
CREATE INDEX idx_model_usage_event_occurred        ON model_usage_event(occurred_at);
CREATE INDEX idx_model_usage_event_ws_occurred     ON model_usage_event(workspace_id, occurred_at);
CREATE INDEX idx_model_usage_event_agent_occurred  ON model_usage_event(workspace_agent_id, occurred_at);

-- Observed intervals per source and scope. The absence of any row for a scope
-- is what 'none' coverage means, so `state` carries only the two observed
-- values; gaps are represented by NOT storing an interval, never by a row.
CREATE TABLE model_usage_coverage (
    id                 TEXT PRIMARY KEY NOT NULL,
    workspace_id       TEXT,
    workspace_agent_id TEXT,
    source_kind        TEXT NOT NULL,
    interval_start     TEXT NOT NULL,
    interval_end       TEXT NOT NULL,
    state              TEXT NOT NULL CHECK (state IN ('complete', 'partial')),
    collector_version  TEXT NOT NULL,
    -- Why an interval is only PARTIAL, when the collector knows. The reserved
    -- code 'unsupported_source' is what makes `usage.overview`'s
    -- `coverage.unsupportedSources` a derived fact instead of a hardcoded
    -- empty list: a collector that meets a source shape it cannot import
    -- records the interval it looked at as partial with this code, and the
    -- query reports that source_kind as unsupported for the scope.
    diagnostic_code    TEXT,
    last_verified_at   TEXT NOT NULL,
    CHECK (interval_end >= interval_start)
);

CREATE INDEX idx_model_usage_coverage_scope
    ON model_usage_coverage(workspace_id, source_kind, interval_start, interval_end);

-- Per-source-file importer cursor. Written in the SAME transaction as the
-- events it produced, so a crash before commit replays safely.
CREATE TABLE model_usage_cursor (
    id                 TEXT PRIMARY KEY NOT NULL,
    source_kind        TEXT    NOT NULL,
    source_session_id  TEXT    NOT NULL,
    -- Stable identity of the file itself, so a rotation/replacement is
    -- detectable without trusting the path alone.
    path_fingerprint   TEXT    NOT NULL,
    byte_offset        INTEGER NOT NULL CHECK (byte_offset     >= 0),
    observed_length    INTEGER NOT NULL CHECK (observed_length >= 0),
    collector_version  TEXT    NOT NULL,
    workspace_id       TEXT,
    workspace_agent_id TEXT,
    verified_owner     TEXT,
    verified_cwd       TEXT,
    -- Bounded parser continuation metadata (never raw transcript text).
    parser_state       TEXT,
    last_verified_at   TEXT    NOT NULL,
    UNIQUE (source_kind, path_fingerprint)
);

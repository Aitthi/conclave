-- H1 shadow-economics (plan docs/superpowers/plans/2026-07-12-hybrid-h1-shadow-economics.md,
-- section "Persistence: migration 0024 and repository"): one terminal row per
-- admitted H1 summary sample. Global constraint #7 (Privacy): only bounded
-- labels, counts, versions, and SHA-256 fingerprints ever land here — never a
-- request body, source text, summary string, prompt, header, or credential.
--
-- Numeric fields for stages the pipeline never reached before a terminal
-- outcome are NULL, never 0 — a 0 is a real measured value.
CREATE TABLE IF NOT EXISTS proxy_summary_metric (
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- identity
    created_at TEXT NOT NULL,
    campaign_id TEXT NOT NULL,
    conversation_hash TEXT NOT NULL,
    model TEXT NOT NULL,
    response_model TEXT,
    method_version TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    price_version TEXT NOT NULL,

    -- boundary
    checkpoint_id TEXT,
    source_boundary_hash TEXT,
    summary_hash TEXT,
    earliest_changed_msg INTEGER,
    runtime_prefix_messages INTEGER,
    tail_start_msg INTEGER,
    tail_target_tokens INTEGER NOT NULL,
    source_count INTEGER,
    protected_result_count INTEGER,

    -- counts
    a_tokens INTEGER,
    b_tokens INTEGER,
    c_tokens INTEGER,
    r_tokens INTEGER,
    s_h_tokens INTEGER,
    q_h REAL,
    summary_tokens INTEGER,
    tombstone_tokens INTEGER,
    projected_post_tokens INTEGER,
    byte_est_tokens INTEGER NOT NULL,
    tail_count_calls INTEGER,
    plateau_turns INTEGER,

    -- generation usage
    gen_input_tokens INTEGER,
    gen_cache_creation_tokens INTEGER,
    gen_cache_read_tokens INTEGER,
    gen_output_tokens INTEGER,

    -- economics: captured price schedule (always known once a campaign admits
    -- the job) plus per-row tier selection and derived spend (known only once
    -- measured).
    standard_input_usd_per_mtok REAL NOT NULL,
    standard_cache_write_usd_per_mtok REAL NOT NULL,
    standard_cache_read_usd_per_mtok REAL NOT NULL,
    standard_output_usd_per_mtok REAL NOT NULL,
    long_input_usd_per_mtok REAL NOT NULL,
    long_cache_write_usd_per_mtok REAL NOT NULL,
    long_cache_read_usd_per_mtok REAL NOT NULL,
    long_output_usd_per_mtok REAL NOT NULL,
    long_context_threshold INTEGER NOT NULL,
    forward_price_tier TEXT CHECK (forward_price_tier IS NULL OR forward_price_tier IN ('standard', 'long')),
    generation_price_tier TEXT CHECK (generation_price_tier IS NULL OR generation_price_tier IN ('standard', 'long')),
    c_gen_usd REAL,
    n_h REAL,
    meets_low_water INTEGER CHECK (meets_low_water IS NULL OR meets_low_water IN (0, 1)),
    meets_two_turn INTEGER CHECK (meets_two_turn IS NULL OR meets_two_turn IN (0, 1)),

    -- terminal state. `outcome` is the top-level bucket; `failure_stage`
    -- further qualifies count_failure/no_candidate/generation_failure/
    -- projection_rejected/metric_invalid rows; `error_type` carries an
    -- allowlisted upstream error.type for count_failure/generation_failure
    -- rows only. All three allowlists are content-free by construction.
    outcome TEXT NOT NULL CHECK (outcome IN (
        'disarmed', 'below_ceiling', 'tail_boundary_failure', 'count_failure',
        'no_candidate', 'generation_failure', 'projection_rejected',
        'metric_invalid', 'measured'
    )),
    failure_stage TEXT CHECK (failure_stage IS NULL OR failure_stage IN (
        'count_a', 'count_b', 'count_c',
        'tool_use', 'non_text', 'empty_text', 'missing_usage', 'timeout',
        'redirect', 'non_2xx', 'decode_error',
        'messages_not_array', 'no_eligible_results', 'replacement_did_not_shrink',
        'structural_validation', 'plan_does_not_match_messages',
        'r_zero', 's_h_zero', 's_h_gt_r'
    )),
    error_type TEXT CHECK (error_type IS NULL OR error_type IN (
        'invalid_request_error', 'authentication_error', 'permission_error',
        'not_found_error', 'request_too_large', 'rate_limit_error',
        'api_error', 'overloaded_error'
    ))
);

CREATE INDEX IF NOT EXISTS idx_proxy_summary_metric_created_at
    ON proxy_summary_metric(created_at);
CREATE INDEX IF NOT EXISTS idx_proxy_summary_metric_campaign_conversation
    ON proxy_summary_metric(campaign_id, conversation_hash);
CREATE INDEX IF NOT EXISTS idx_proxy_summary_metric_campaign_outcome
    ON proxy_summary_metric(campaign_id, outcome);

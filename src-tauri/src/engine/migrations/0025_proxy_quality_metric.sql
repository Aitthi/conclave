-- H2 shadow-quality validation (plan docs/superpowers/plans/
-- 2026-07-12-hybrid-h2-shadow-quality.md, section "Persistence: migration 0025
-- and repository"): one terminal row per reserved quality case, plus the
-- bounded human-audit verdict table. Global constraint #8 (Privacy): counts,
-- bounded enums, booleans, model/prompt versions, usage totals, and SHA-256
-- hashes only — no probes, plans, summaries, judge output, credentials, or
-- raw upstream errors. Raw-shaped text columns are forbidden; every free-text
-- column carries a shape or length CHECK. The in-memory credential/upstream
-- identity hash from the evaluator preflight deliberately has NO column here.
--
-- failure_stage/failure_kind/error_type allowlists are pinned to Lane B's
-- ACTUAL merged QualityClientError vocabulary (runtime/quality.rs:
-- QualityCallStage::as_str + every `new(stage, ...)` kind literal + the eight
-- count_tokens KNOWN_ERROR_TYPES, populated only when kind = 'non_2xx') —
-- read from the code at lane-B SHA 84bdc09d, not from plan prose (the H1
-- allowlist-from-prose lesson).
CREATE TABLE IF NOT EXISTS proxy_quality_metric (
    id INTEGER PRIMARY KEY AUTOINCREMENT,

    -- identity
    created_at TEXT NOT NULL,
    quality_campaign_id TEXT NOT NULL CHECK (
        length(quality_campaign_id) = 36 AND quality_campaign_id NOT GLOB '*[^0-9a-f-]*'
    ),
    h1_campaign_id TEXT NOT NULL CHECK (
        length(h1_campaign_id) = 36 AND h1_campaign_id NOT GLOB '*[^0-9a-f-]*'
    ),
    -- One reserved case persists exactly one outcome (Global constraint #9):
    -- UNIQUE makes a second terminal row for the same case a hard DB error.
    case_id TEXT NOT NULL UNIQUE CHECK (
        length(case_id) = 36 AND case_id NOT GLOB '*[^0-9a-f-]*'
    ),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('live', 'fixture')),
    fixture_id TEXT CHECK (fixture_id IS NULL OR length(fixture_id) BETWEEN 1 AND 128),
    fixture_family TEXT CHECK (fixture_family IS NULL OR length(fixture_family) BETWEEN 1 AND 128),

    -- seven persisted stratum tags (runtime::quality::QualityTag::ALL)
    tag_side_effecting_output INTEGER NOT NULL CHECK (tag_side_effecting_output IN (0, 1)),
    tag_long_log INTEGER NOT NULL CHECK (tag_long_log IN (0, 1)),
    tag_exact_error INTEGER NOT NULL CHECK (tag_exact_error IN (0, 1)),
    tag_rejected_alternative INTEGER NOT NULL CHECK (tag_rejected_alternative IN (0, 1)),
    tag_parallel_tool_cycle INTEGER NOT NULL CHECK (tag_parallel_tool_cycle IN (0, 1)),
    tag_prompt_like_tool_text INTEGER NOT NULL CHECK (tag_prompt_like_tool_text IN (0, 1)),
    tag_mutation_or_open_work INTEGER NOT NULL CHECK (tag_mutation_or_open_work IN (0, 1)),

    -- boundary hashes (SHA-256 lowercase hex; checkpoint_id is ctxopt's
    -- 16-hex stable identity key). Stage-dependent hashes are NULL for
    -- stages the case never reached — never a raw value, never zero-as-missing.
    conversation_hash TEXT NOT NULL CHECK (
        length(conversation_hash) = 64 AND conversation_hash NOT GLOB '*[^0-9a-f]*'
    ),
    checkpoint_id TEXT CHECK (
        checkpoint_id IS NULL
        OR (length(checkpoint_id) = 16 AND checkpoint_id NOT GLOB '*[^0-9a-f]*')
    ),
    source_boundary_hash TEXT CHECK (
        source_boundary_hash IS NULL
        OR (length(source_boundary_hash) = 64 AND source_boundary_hash NOT GLOB '*[^0-9a-f]*')
    ),
    summary_hash TEXT CHECK (
        summary_hash IS NULL
        OR (length(summary_hash) = 64 AND summary_hash NOT GLOB '*[^0-9a-f]*')
    ),
    probe_set_hash TEXT CHECK (
        probe_set_hash IS NULL
        OR (length(probe_set_hash) = 64 AND probe_set_hash NOT GLOB '*[^0-9a-f]*')
    ),
    original_plan_hash TEXT CHECK (
        original_plan_hash IS NULL
        OR (length(original_plan_hash) = 64 AND original_plan_hash NOT GLOB '*[^0-9a-f]*')
    ),
    projected_plan_hash TEXT CHECK (
        projected_plan_hash IS NULL
        OR (length(projected_plan_hash) = 64 AND projected_plan_hash NOT GLOB '*[^0-9a-f]*')
    ),
    judge_hash TEXT CHECK (
        judge_hash IS NULL
        OR (length(judge_hash) = 64 AND judge_hash NOT GLOB '*[^0-9a-f]*')
    ),

    -- versions/models. Version columns are written from the compiled Lane A
    -- constants by the sole insert API, so a row always names the exact code
    -- that produced it; models are config-supplied and length-capped.
    task_model TEXT NOT NULL CHECK (length(task_model) BETWEEN 1 AND 128),
    summarizer_response_model TEXT CHECK (
        summarizer_response_model IS NULL OR length(summarizer_response_model) BETWEEN 1 AND 128
    ),
    evaluator_response_model TEXT CHECK (
        evaluator_response_model IS NULL OR length(evaluator_response_model) BETWEEN 1 AND 128
    ),
    method_version TEXT NOT NULL CHECK (length(method_version) BETWEEN 1 AND 64),
    rubric_version TEXT NOT NULL CHECK (length(rubric_version) BETWEEN 1 AND 64),
    probe_prompt_version TEXT NOT NULL CHECK (length(probe_prompt_version) BETWEEN 1 AND 64),
    faithfulness_prompt_version TEXT NOT NULL CHECK (
        length(faithfulness_prompt_version) BETWEEN 1 AND 64
    ),
    replay_prompt_version TEXT NOT NULL CHECK (length(replay_prompt_version) BETWEEN 1 AND 64),
    judge_prompt_version TEXT NOT NULL CHECK (length(judge_prompt_version) BETWEEN 1 AND 64),

    -- structure/faithfulness (NULL until the stage that produces them)
    structural_pass INTEGER CHECK (structural_pass IS NULL OR structural_pass IN (0, 1)),
    claims_total INTEGER CHECK (claims_total IS NULL OR claims_total >= 0),
    claims_supported INTEGER CHECK (claims_supported IS NULL OR claims_supported >= 0),
    claims_unsupported INTEGER CHECK (claims_unsupported IS NULL OR claims_unsupported >= 0),
    critical_hallucinations INTEGER CHECK (
        critical_hallucinations IS NULL OR critical_hallucinations >= 0
    ),
    noncritical_hallucinations INTEGER CHECK (
        noncritical_hallucinations IS NULL OR noncritical_hallucinations >= 0
    ),
    critical_omissions INTEGER CHECK (critical_omissions IS NULL OR critical_omissions >= 0),
    noncritical_omissions INTEGER CHECK (
        noncritical_omissions IS NULL OR noncritical_omissions >= 0
    ),
    probes_total INTEGER CHECK (probes_total IS NULL OR probes_total >= 0),
    probes_retained INTEGER CHECK (probes_retained IS NULL OR probes_retained >= 0),
    probe_recall REAL CHECK (probe_recall IS NULL OR (probe_recall >= 0.0 AND probe_recall <= 1.0)),

    -- behavior (NULL until replay/judge complete)
    original_correct INTEGER CHECK (original_correct IS NULL OR original_correct IN (0, 1)),
    original_constraint_adherent INTEGER CHECK (
        original_constraint_adherent IS NULL OR original_constraint_adherent IN (0, 1)
    ),
    original_next_action_match INTEGER CHECK (
        original_next_action_match IS NULL OR original_next_action_match IN (0, 1)
    ),
    projected_correct INTEGER CHECK (projected_correct IS NULL OR projected_correct IN (0, 1)),
    projected_constraint_adherent INTEGER CHECK (
        projected_constraint_adherent IS NULL OR projected_constraint_adherent IN (0, 1)
    ),
    projected_next_action_match INTEGER CHECK (
        projected_next_action_match IS NULL OR projected_next_action_match IN (0, 1)
    ),
    original_pass INTEGER CHECK (original_pass IS NULL OR original_pass IN (0, 1)),
    projected_pass INTEGER CHECK (projected_pass IS NULL OR projected_pass IN (0, 1)),
    comparison TEXT CHECK (
        comparison IS NULL
        OR comparison IN ('original_win', 'tie', 'projected_win', 'both_fail')
    ),

    -- usage: four provider buckets per role, six roles. NULL for calls that
    -- never happened.
    preflight_input_tokens INTEGER,
    preflight_cache_creation_tokens INTEGER,
    preflight_cache_read_tokens INTEGER,
    preflight_output_tokens INTEGER,
    probe_input_tokens INTEGER,
    probe_cache_creation_tokens INTEGER,
    probe_cache_read_tokens INTEGER,
    probe_output_tokens INTEGER,
    faithfulness_input_tokens INTEGER,
    faithfulness_cache_creation_tokens INTEGER,
    faithfulness_cache_read_tokens INTEGER,
    faithfulness_output_tokens INTEGER,
    original_replay_input_tokens INTEGER,
    original_replay_cache_creation_tokens INTEGER,
    original_replay_cache_read_tokens INTEGER,
    original_replay_output_tokens INTEGER,
    projected_replay_input_tokens INTEGER,
    projected_replay_cache_creation_tokens INTEGER,
    projected_replay_cache_read_tokens INTEGER,
    projected_replay_output_tokens INTEGER,
    judge_input_tokens INTEGER,
    judge_cache_creation_tokens INTEGER,
    judge_cache_read_tokens INTEGER,
    judge_output_tokens INTEGER,

    -- terminal state. failure_stage = which quality call (Lane B
    -- QualityCallStage); failure_kind = Lane B's bounded kind label;
    -- error_type = allowlisted Anthropic error.type, present ONLY when
    -- failure_kind = 'non_2xx' (Lane B drops everything else to None).
    outcome TEXT NOT NULL CHECK (outcome IN (
        'preflight_ok', 'preflight_failure', 'structural_failure',
        'call_failure', 'disarmed', 'completed'
    )),
    failure_stage TEXT CHECK (failure_stage IS NULL OR failure_stage IN (
        'preflight', 'probe', 'faithfulness', 'original_replay',
        'projected_replay', 'judge'
    )),
    failure_kind TEXT CHECK (failure_kind IS NULL OR failure_kind IN (
        'missing_auth', 'timeout', 'transport', 'redirect', 'non_2xx',
        'decode', 'missing_model', 'missing_content', 'non_text_content',
        'empty_text', 'missing_usage',
        'schema_json', 'schema_shape', 'schema_bounds', 'schema_reconcile',
        'schema_citation', 'schema_duplicate', 'schema_category'
    )),
    error_type TEXT CHECK (error_type IS NULL OR error_type IN (
        'invalid_request_error', 'authentication_error', 'permission_error',
        'not_found_error', 'request_too_large', 'rate_limit_error',
        'api_error', 'overloaded_error'
    )),
    CHECK (error_type IS NULL OR failure_kind = 'non_2xx'),
    -- fixture identity present exactly when the case is a fixture case.
    CHECK ((source_kind = 'fixture') = (fixture_id IS NOT NULL)),
    CHECK ((source_kind = 'fixture') = (fixture_family IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS idx_proxy_quality_metric_created_at
    ON proxy_quality_metric(created_at);
CREATE INDEX IF NOT EXISTS idx_proxy_quality_metric_campaign_outcome
    ON proxy_quality_metric(quality_campaign_id, outcome);
CREATE INDEX IF NOT EXISTS idx_proxy_quality_metric_campaign_conversation
    ON proxy_quality_metric(quality_campaign_id, conversation_hash);

-- Human-audit verdicts (plan §"Human audit without raw persistence"): one
-- bounded verdict per audited FIXTURE case. UNIQUE(quality_metric_id) makes a
-- verdict one-shot at the DB layer — a rerun means a new campaign, never an
-- edited row — and the repo exposes no update API. Fixture-only is enforced by
-- the sole insert API (live rows are rejected before SQL).
CREATE TABLE IF NOT EXISTS proxy_quality_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    quality_metric_id INTEGER NOT NULL UNIQUE REFERENCES proxy_quality_metric(id),
    audit_bucket TEXT NOT NULL CHECK (
        audit_bucket IN ('accepted', 'rejected', 'near_threshold')
    ),
    verdict TEXT NOT NULL CHECK (verdict IN ('agree', 'disagree')),
    rubric_version TEXT NOT NULL CHECK (length(rubric_version) BETWEEN 1 AND 64)
);

CREATE INDEX IF NOT EXISTS idx_proxy_quality_audit_metric
    ON proxy_quality_audit(quality_metric_id);

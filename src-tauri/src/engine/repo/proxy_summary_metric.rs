//! H1 shadow-economics metric contract (plan
//! `docs/superpowers/plans/2026-07-12-hybrid-h1-shadow-economics.md`, section
//! "Persistence: migration 0024 and repository") for the app-global context
//! proxy.
//!
//! One terminal row per admitted H1 summary sample. Global constraint #7
//! (Privacy): only bounded labels, counts, versions, and SHA-256 fingerprints
//! ever land here — never a request body, source text, summary string,
//! prompt, header, or credential. There is deliberately no column in this
//! schema shaped to hold any of those (see
//! `schema_column_list_matches_the_bounded_contract` below).
//!
//! [`insert_terminal`] is the only write API. The outcome is a Rust enum
//! (Global constraint #6), so a call site cannot express two terminal states
//! for the same row, and a rejected/no-candidate stage can never carry the
//! free-text payload some `ctxopt::summary::SummaryError` variants hold
//! (`StructuralValidation(String)`, `ReplacementDidNotShrink { tool_use_id }`)
//! — only the variant identity survives as a fixed label.

use chrono::{Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

/// Upstream Anthropic `error.type` allowlist. Mirrors
/// `runtime::count_tokens::KNOWN_ERROR_TYPES` (rulings e376fec5 + f8651210,
/// challenge fda10918) but is duplicated here rather than imported: this
/// lane's file boundary is migration/db.rs/repo only and must not reach into
/// `runtime/count_tokens.rs`. Any other value is rejected before the query
/// even runs — the DB CHECK constraint is the final backstop for a caller
/// that bypasses this function.
const KNOWN_ERROR_TYPES: &[&str] = &[
    "invalid_request_error",
    "authentication_error",
    "permission_error",
    "not_found_error",
    "request_too_large",
    "rate_limit_error",
    "api_error",
    "overloaded_error",
];

/// count_tokens failure sub-stage (plan steps 2 and 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountFailureStage {
    CountA,
    CountB,
    CountC,
}

impl CountFailureStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::CountA => "count_a",
            Self::CountB => "count_b",
            Self::CountC => "count_c",
        }
    }
}

/// Bounded projection of `ctxopt::summary::SummaryError` (H0), used for both
/// `no_candidate` (plan step 4, `plan_summary_span`) and `projection_rejected`
/// (plan step 7, `build_summary_projection`). Deliberately drops every
/// variant's payload — `ReplacementDidNotShrink { tool_use_id }` and
/// `StructuralValidation(String)` can carry tool-controlled or
/// upstream-echoed text, and only the variant identity is privacy-safe to
/// persist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanRejectStage {
    MessagesNotArray,
    NoEligibleResults,
    ReplacementDidNotShrink,
    StructuralValidation,
    PlanDoesNotMatchMessages,
}

impl PlanRejectStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::MessagesNotArray => "messages_not_array",
            Self::NoEligibleResults => "no_eligible_results",
            Self::ReplacementDidNotShrink => "replacement_did_not_shrink",
            Self::StructuralValidation => "structural_validation",
            Self::PlanDoesNotMatchMessages => "plan_does_not_match_messages",
        }
    }
}

/// Generation-call failure sub-stage. Built from Lane A's actual merged
/// `runtime::summary::SummaryClientError::kind()` vocabulary (commit
/// `0a11dc2`, verified directly against `generate_summary`'s call sites —
/// not the plan's step-6 prose, which this lane could not cross-check against
/// Lane A's real implementation at the time it was written):
/// `missing_auth`, `timeout`, `transport`, `redirect`, the 8 allowlisted
/// Anthropic error types (`SanitizedErrorType::Known`) plus
/// `http_unknown_type`/`http_unparsed` (all four map to `Non2xx` — the
/// allowlisted type, when known, is recorded separately in `error_type`),
/// `decode`, `missing_model`, `missing_content`, `non_text_content`, `empty_text`,
/// `missing_usage`. There is no reachable `tool_use` case: Lane A's block-type
/// check treats ANY non-`"text"` content block (a real tool_use block would
/// never occur given `tool_choice:"none"`, ruling `0807943e`) as
/// `non_text_content`, so a dedicated `ToolUse` bucket was dead — removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationFailureStage {
    MissingAuth,
    Transport,
    Timeout,
    Redirect,
    /// Any non-2xx response: `error_type` carries the allowlisted Anthropic
    /// type when the upstream body parsed as one of the 8 known types;
    /// `None` for `http_unknown_type`/`http_unparsed`.
    Non2xx,
    /// Lane A's `decode` kind (response body failed to parse as JSON).
    DecodeError,
    MissingModel,
    MissingContent,
    /// Lane A's `non_text_content` kind (a content block whose `type` was
    /// not `"text"`).
    NonText,
    EmptyText,
    MissingUsage,
}

impl GenerationFailureStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingAuth => "missing_auth",
            Self::Transport => "transport",
            Self::Timeout => "timeout",
            Self::Redirect => "redirect",
            Self::Non2xx => "non_2xx",
            Self::DecodeError => "decode_error",
            Self::MissingModel => "missing_model",
            Self::MissingContent => "missing_content",
            Self::NonText => "non_text",
            Self::EmptyText => "empty_text",
            Self::MissingUsage => "missing_usage",
        }
    }
}

/// Arithmetic-invalid sub-stage (plan step 9: `R=0`, `S_h=0`, `S_h>R`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricInvalidStage {
    RZero,
    SHZero,
    SHGreaterThanR,
}

impl MetricInvalidStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::RZero => "r_zero",
            Self::SHZero => "s_h_zero",
            Self::SHGreaterThanR => "s_h_gt_r",
        }
    }
}

/// Price tier selected independently for the forwarded turn and the
/// generation call (plan step 9: "one may be long while the other is
/// standard").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceTier {
    Standard,
    Long,
}

impl PriceTier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Long => "long",
        }
    }
}

/// The row's single terminal state (Global constraints #6/#7). Exactly one
/// variant is constructible per [`insert_terminal`] call — there is no way to
/// express "this row is both `disarmed` and `measured`", and a rejection
/// variant physically cannot carry more than its bounded stage label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryOutcome {
    Disarmed,
    BelowCeiling,
    TailBoundaryFailure,
    CountFailure(CountFailureStage),
    NoCandidate(PlanRejectStage),
    GenerationFailure(GenerationFailureStage),
    ProjectionRejected(PlanRejectStage),
    MetricInvalid(MetricInvalidStage),
    Measured,
}

impl SummaryOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Disarmed => "disarmed",
            Self::BelowCeiling => "below_ceiling",
            Self::TailBoundaryFailure => "tail_boundary_failure",
            Self::CountFailure(_) => "count_failure",
            Self::NoCandidate(_) => "no_candidate",
            Self::GenerationFailure(_) => "generation_failure",
            Self::ProjectionRejected(_) => "projection_rejected",
            Self::MetricInvalid(_) => "metric_invalid",
            Self::Measured => "measured",
        }
    }

    fn stage_label(self) -> Option<&'static str> {
        match self {
            Self::CountFailure(s) => Some(s.as_str()),
            Self::NoCandidate(s) | Self::ProjectionRejected(s) => Some(s.as_str()),
            Self::GenerationFailure(s) => Some(s.as_str()),
            Self::MetricInvalid(s) => Some(s.as_str()),
            Self::Disarmed | Self::BelowCeiling | Self::TailBoundaryFailure | Self::Measured => {
                None
            }
        }
    }

    pub fn is_measured(self) -> bool {
        matches!(self, Self::Measured)
    }
}

/// One terminal H1 sample. Field grouping mirrors the plan's schema section
/// (identity / boundary / counts / generation usage / economics / terminal
/// state). Nullable fields are stages the pipeline may terminate before
/// reaching — never encode "not reached" as `0`.
///
/// `plateau_turns` is intentionally absent: [`insert_terminal`] derives it
/// transactionally from the previous `measured` row sharing
/// `(campaign_id, conversation_hash, source_boundary_hash)`.
pub struct SummaryMetricInsert {
    // identity
    pub created_at: String,
    pub campaign_id: String,
    pub conversation_hash: String,
    pub model: String,
    pub response_model: Option<String>,
    pub method_version: String,
    pub prompt_version: String,
    pub price_version: String,

    // boundary
    pub checkpoint_id: Option<String>,
    pub source_boundary_hash: Option<String>,
    pub summary_hash: Option<String>,
    pub earliest_changed_msg: Option<i64>,
    pub runtime_prefix_messages: Option<i64>,
    pub tail_start_msg: Option<i64>,
    pub tail_target_tokens: i64,
    pub source_count: Option<i64>,
    pub protected_result_count: Option<i64>,

    // counts
    pub a_tokens: Option<i64>,
    pub b_tokens: Option<i64>,
    pub c_tokens: Option<i64>,
    pub r_tokens: Option<i64>,
    pub s_h_tokens: Option<i64>,
    pub q_h: Option<f64>,
    pub summary_tokens: Option<i64>,
    pub tombstone_tokens: Option<i64>,
    pub projected_post_tokens: Option<i64>,
    pub byte_est_tokens: i64,
    pub tail_count_calls: Option<i64>,

    // generation usage
    pub gen_input_tokens: Option<i64>,
    pub gen_cache_creation_tokens: Option<i64>,
    pub gen_cache_read_tokens: Option<i64>,
    pub gen_output_tokens: Option<i64>,

    // economics
    pub standard_input_usd_per_mtok: f64,
    pub standard_cache_write_usd_per_mtok: f64,
    pub standard_cache_read_usd_per_mtok: f64,
    pub standard_output_usd_per_mtok: f64,
    pub long_input_usd_per_mtok: f64,
    pub long_cache_write_usd_per_mtok: f64,
    pub long_cache_read_usd_per_mtok: f64,
    pub long_output_usd_per_mtok: f64,
    pub long_context_threshold: i64,
    pub forward_price_tier: Option<PriceTier>,
    pub generation_price_tier: Option<PriceTier>,
    pub c_gen_usd: Option<f64>,
    pub n_h: Option<f64>,
    pub meets_low_water: Option<bool>,
    pub meets_two_turn: Option<bool>,

    // terminal state
    pub outcome: SummaryOutcome,
    /// Upstream `error.type` for `CountFailure`/`GenerationFailure` rows
    /// only; must be one of [`KNOWN_ERROR_TYPES`] or `None`. Anything else is
    /// rejected before the row is ever written.
    pub error_type: Option<String>,
}

/// SHA-256 hex digest length (`conversation_hash`, `source_boundary_hash`,
/// `summary_hash` — plan §"Persistence": "SHA-256 of the first message
/// bytes" / "SHA-256 of the accepted summary").
const SHA256_HEX_LEN: usize = 64;

/// `ctxopt::summary::stable_checkpoint_id`'s exact format: a 16-hex-char
/// FNV-1a identity key (`format!("{hash:016x}")`). Not a security digest —
/// still hex-only, so a non-hex value is a strong signal of raw content.
const CHECKPOINT_ID_HEX_LEN: usize = 16;

/// `true` iff `s` is exactly `len` lowercase ASCII hex digits. A hash/id
/// column holding anything else — spaces, punctuation, mixed case, forged
/// delimiters — is a strong signal the caller passed real content (a raw
/// summary, a tool-output snippet) instead of its hash.
fn is_lowercase_hex_of_len(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Insert one terminal row. Requires exactly one [`SummaryOutcome`] (there is
/// no other way to construct a row). For `Measured` outcomes, derives
/// `plateau_turns` transactionally: the same `(campaign_id, conversation_hash,
/// source_boundary_hash)` as the immediately preceding `measured` row for that
/// conversation increments the plateau; a changed boundary resets it to zero
/// (plan step 10).
///
/// Fail-closed shape validation (Mellow review finding 1, HIGH — privacy):
/// `conversation_hash`/`source_boundary_hash`/`summary_hash` must be 64-char
/// lowercase SHA-256 hex digests and `checkpoint_id` must be ctxopt's 16-char
/// lowercase hex identity key. This is the backstop against a caller
/// accidentally passing real content (a raw summary, a tool-output snippet)
/// into a column that must only ever hold its hash.
pub async fn insert_terminal(pool: &SqlitePool, m: SummaryMetricInsert) -> sqlx::Result<()> {
    if !is_lowercase_hex_of_len(&m.conversation_hash, SHA256_HEX_LEN) {
        return Err(sqlx::Error::Protocol(
            "[proxy_summary_metric] conversation_hash must be a 64-char lowercase SHA-256 hex \
             digest"
                .into(),
        ));
    }
    if let Some(h) = m.summary_hash.as_deref() {
        if !is_lowercase_hex_of_len(h, SHA256_HEX_LEN) {
            return Err(sqlx::Error::Protocol(
                "[proxy_summary_metric] summary_hash must be a 64-char lowercase SHA-256 hex \
                 digest"
                    .into(),
            ));
        }
    }
    if let Some(h) = m.source_boundary_hash.as_deref() {
        if !is_lowercase_hex_of_len(h, SHA256_HEX_LEN) {
            return Err(sqlx::Error::Protocol(
                "[proxy_summary_metric] source_boundary_hash must be a 64-char lowercase \
                 SHA-256 hex digest"
                    .into(),
            ));
        }
    }
    if let Some(id) = m.checkpoint_id.as_deref() {
        if !is_lowercase_hex_of_len(id, CHECKPOINT_ID_HEX_LEN) {
            return Err(sqlx::Error::Protocol(
                "[proxy_summary_metric] checkpoint_id must be a 16-char lowercase hex identity \
                 key"
                .into(),
            ));
        }
    }
    if let Some(et) = m.error_type.as_deref() {
        if !KNOWN_ERROR_TYPES.contains(&et) {
            return Err(sqlx::Error::Protocol(format!(
                "[proxy_summary_metric] error_type `{et}` is not in the allowlist"
            )));
        }
    }

    let mut tx = pool.begin().await?;

    let plateau_turns: Option<i64> = if m.outcome.is_measured() {
        let boundary = m.source_boundary_hash.as_deref().ok_or_else(|| {
            sqlx::Error::Protocol(
                "[proxy_summary_metric] a `measured` row requires source_boundary_hash".into(),
            )
        })?;
        let prev: Option<(Option<String>, i64)> = sqlx::query_as(
            "SELECT source_boundary_hash, plateau_turns FROM proxy_summary_metric \
             WHERE campaign_id = ?1 AND conversation_hash = ?2 AND outcome = 'measured' \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(&m.campaign_id)
        .bind(&m.conversation_hash)
        .fetch_optional(&mut *tx)
        .await?;
        Some(match prev {
            Some((Some(prev_boundary), prev_plateau)) if prev_boundary == boundary => {
                prev_plateau + 1
            }
            _ => 0,
        })
    } else {
        None
    };

    sqlx::query(
        "INSERT INTO proxy_summary_metric (\
         created_at, campaign_id, conversation_hash, model, response_model, \
         method_version, prompt_version, price_version, \
         checkpoint_id, source_boundary_hash, summary_hash, earliest_changed_msg, \
         runtime_prefix_messages, tail_start_msg, tail_target_tokens, source_count, \
         protected_result_count, \
         a_tokens, b_tokens, c_tokens, r_tokens, s_h_tokens, q_h, summary_tokens, \
         tombstone_tokens, projected_post_tokens, byte_est_tokens, tail_count_calls, \
         plateau_turns, \
         gen_input_tokens, gen_cache_creation_tokens, gen_cache_read_tokens, gen_output_tokens, \
         standard_input_usd_per_mtok, standard_cache_write_usd_per_mtok, \
         standard_cache_read_usd_per_mtok, standard_output_usd_per_mtok, \
         long_input_usd_per_mtok, long_cache_write_usd_per_mtok, \
         long_cache_read_usd_per_mtok, long_output_usd_per_mtok, long_context_threshold, \
         forward_price_tier, generation_price_tier, c_gen_usd, n_h, \
         meets_low_water, meets_two_turn, \
         outcome, failure_stage, error_type) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, \
         ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, \
         ?35, ?36, ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44, ?45, ?46, ?47, ?48, ?49, ?50, ?51)",
    )
    .bind(m.created_at)
    .bind(m.campaign_id)
    .bind(m.conversation_hash)
    .bind(m.model)
    .bind(m.response_model)
    .bind(m.method_version)
    .bind(m.prompt_version)
    .bind(m.price_version)
    .bind(m.checkpoint_id)
    .bind(m.source_boundary_hash)
    .bind(m.summary_hash)
    .bind(m.earliest_changed_msg)
    .bind(m.runtime_prefix_messages)
    .bind(m.tail_start_msg)
    .bind(m.tail_target_tokens)
    .bind(m.source_count)
    .bind(m.protected_result_count)
    .bind(m.a_tokens)
    .bind(m.b_tokens)
    .bind(m.c_tokens)
    .bind(m.r_tokens)
    .bind(m.s_h_tokens)
    .bind(m.q_h)
    .bind(m.summary_tokens)
    .bind(m.tombstone_tokens)
    .bind(m.projected_post_tokens)
    .bind(m.byte_est_tokens)
    .bind(m.tail_count_calls)
    .bind(plateau_turns)
    .bind(m.gen_input_tokens)
    .bind(m.gen_cache_creation_tokens)
    .bind(m.gen_cache_read_tokens)
    .bind(m.gen_output_tokens)
    .bind(m.standard_input_usd_per_mtok)
    .bind(m.standard_cache_write_usd_per_mtok)
    .bind(m.standard_cache_read_usd_per_mtok)
    .bind(m.standard_output_usd_per_mtok)
    .bind(m.long_input_usd_per_mtok)
    .bind(m.long_cache_write_usd_per_mtok)
    .bind(m.long_cache_read_usd_per_mtok)
    .bind(m.long_output_usd_per_mtok)
    .bind(m.long_context_threshold)
    .bind(m.forward_price_tier.map(PriceTier::as_str))
    .bind(m.generation_price_tier.map(PriceTier::as_str))
    .bind(m.c_gen_usd)
    .bind(m.n_h)
    .bind(m.meets_low_water.map(i64::from))
    .bind(m.meets_two_turn.map(i64::from))
    .bind(m.outcome.label())
    .bind(m.outcome.stage_label())
    .bind(m.error_type)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Per-`(price_version, model)` row count (report grouping).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SummaryGroupCount {
    pub price_version: String,
    pub model: String,
    pub count: i64,
}

/// Per-conversation measured/passing split. No pooled percentage is allowed
/// to hide a conversation whose measured candidates all miss both bars (plan
/// §"SummaryReport") — this is the row-level evidence for that check.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPassCount {
    pub conversation_hash: String,
    pub measured_candidates: i64,
    /// `meets_low_water AND meets_two_turn` count within this conversation.
    pub passing_candidates: i64,
}

/// Campaign-aware H1 economics report.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryReport {
    pub total_admitted: i64,
    pub measured: i64,
    pub disarmed: i64,
    pub below_ceiling: i64,
    pub tail_boundary_failures: i64,
    pub no_candidate: i64,
    pub count_failures: i64,
    pub generation_failures: i64,
    pub projection_rejected: i64,
    pub metric_invalid: i64,
    /// Distinct `conversation_hash` values among `measured` rows only.
    pub distinct_conversations: i64,
    /// `(count_failures + generation_failures) / (measured + count_failures +
    /// generation_failures)`; `0.0` on an empty denominator. This is the
    /// plan-pinned GO-bar formula — `below_ceiling`, `no_candidate`,
    /// `projection_rejected`, `disarmed`, and queue drops never dilute it.
    pub failure_rate: f64,
    /// `measured` rows with real `a_tokens` in `[400_000, 500_000]`.
    pub band_400k_500k: i64,
    pub pct_meets_low_water: f64,
    pub pct_meets_two_turn: f64,
    pub q_h_min: f64,
    pub q_h_median: f64,
    pub q_h_max: f64,
    pub q_h_avg: f64,
    pub n_h_min: f64,
    pub n_h_median: f64,
    pub n_h_max: f64,
    pub n_h_avg: f64,
    pub max_plateau_turns: i64,
    pub gen_input_tokens_total: i64,
    pub gen_cache_creation_tokens_total: i64,
    pub gen_cache_read_tokens_total: i64,
    pub gen_output_tokens_total: i64,
    pub by_price_version_and_model: Vec<SummaryGroupCount>,
    pub conversation_pass_counts: Vec<ConversationPassCount>,
}

/// SQL-computable aggregates (everything except q_h/n_h median, which SQLite
/// lacks — pulled and sorted in Rust below, same pattern as
/// `proxy_checkpoint_metric::report`).
#[derive(sqlx::FromRow)]
struct Agg {
    total_admitted: i64,
    measured: i64,
    disarmed: i64,
    below_ceiling: i64,
    tail_boundary_failures: i64,
    no_candidate: i64,
    count_failures: i64,
    generation_failures: i64,
    projection_rejected: i64,
    metric_invalid: i64,
    distinct_conversations: i64,
    band_400k_500k: i64,
    meets_low_water_count: i64,
    meets_two_turn_count: i64,
    max_plateau_turns: i64,
    gen_input_tokens_total: i64,
    gen_cache_creation_tokens_total: i64,
    gen_cache_read_tokens_total: i64,
    gen_output_tokens_total: i64,
}

fn stats(sorted: &[f64]) -> (f64, f64, f64, f64) {
    let n = sorted.len();
    if n == 0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let median = if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    };
    let avg = sorted.iter().sum::<f64>() / n as f64;
    (sorted[0], median, sorted[n - 1], avg)
}

/// Campaign-aware distributions + the exact H1 numerators/denominators (plan
/// §"SummaryReport"). `campaign_id = None` reports across every campaign;
/// `dropped`/`model-mismatch` counters live on runtime status (Lane C/D), not
/// here.
pub async fn report(
    pool: &SqlitePool,
    since_hours: i64,
    campaign_id: Option<&str>,
) -> sqlx::Result<SummaryReport> {
    let span = Duration::try_hours(since_hours.max(0)).unwrap_or(Duration::MAX);
    let cutoff = Utc::now()
        .checked_sub_signed(span)
        .unwrap_or(chrono::DateTime::<Utc>::MIN_UTC)
        .to_rfc3339();

    let agg = sqlx::query_as::<_, Agg>(
        "SELECT \
           COUNT(*) AS total_admitted, \
           COALESCE(SUM(outcome = 'measured'), 0) AS measured, \
           COALESCE(SUM(outcome = 'disarmed'), 0) AS disarmed, \
           COALESCE(SUM(outcome = 'below_ceiling'), 0) AS below_ceiling, \
           COALESCE(SUM(outcome = 'tail_boundary_failure'), 0) AS tail_boundary_failures, \
           COALESCE(SUM(outcome = 'no_candidate'), 0) AS no_candidate, \
           COALESCE(SUM(outcome = 'count_failure'), 0) AS count_failures, \
           COALESCE(SUM(outcome = 'generation_failure'), 0) AS generation_failures, \
           COALESCE(SUM(outcome = 'projection_rejected'), 0) AS projection_rejected, \
           COALESCE(SUM(outcome = 'metric_invalid'), 0) AS metric_invalid, \
           COUNT(DISTINCT CASE WHEN outcome = 'measured' THEN conversation_hash END) \
             AS distinct_conversations, \
           COALESCE(SUM(outcome = 'measured' AND a_tokens BETWEEN 400000 AND 500000), 0) \
             AS band_400k_500k, \
           COALESCE(SUM(outcome = 'measured' AND meets_low_water = 1), 0) \
             AS meets_low_water_count, \
           COALESCE(SUM(outcome = 'measured' AND meets_two_turn = 1), 0) \
             AS meets_two_turn_count, \
           COALESCE(MAX(plateau_turns), 0) AS max_plateau_turns, \
           COALESCE(SUM(gen_input_tokens), 0) AS gen_input_tokens_total, \
           COALESCE(SUM(gen_cache_creation_tokens), 0) AS gen_cache_creation_tokens_total, \
           COALESCE(SUM(gen_cache_read_tokens), 0) AS gen_cache_read_tokens_total, \
           COALESCE(SUM(gen_output_tokens), 0) AS gen_output_tokens_total \
         FROM proxy_summary_metric \
         WHERE created_at >= ?1 AND (?2 IS NULL OR campaign_id = ?2)",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_one(pool)
    .await?;

    let qs: Vec<f64> = sqlx::query_scalar(
        "SELECT q_h FROM proxy_summary_metric \
         WHERE outcome = 'measured' AND q_h IS NOT NULL AND created_at >= ?1 \
         AND (?2 IS NULL OR campaign_id = ?2) ORDER BY q_h",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_all(pool)
    .await?;

    let ns: Vec<f64> = sqlx::query_scalar(
        "SELECT n_h FROM proxy_summary_metric \
         WHERE outcome = 'measured' AND n_h IS NOT NULL AND created_at >= ?1 \
         AND (?2 IS NULL OR campaign_id = ?2) ORDER BY n_h",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_all(pool)
    .await?;

    let by_price_version_and_model: Vec<SummaryGroupCount> = sqlx::query_as(
        "SELECT price_version, model, COUNT(*) AS count FROM proxy_summary_metric \
         WHERE created_at >= ?1 AND (?2 IS NULL OR campaign_id = ?2) \
         GROUP BY price_version, model ORDER BY price_version, model",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_all(pool)
    .await?;

    let conversation_pass_counts: Vec<ConversationPassCount> = sqlx::query_as(
        "SELECT conversation_hash, COUNT(*) AS measured_candidates, \
           COALESCE(SUM(meets_low_water = 1 AND meets_two_turn = 1), 0) AS passing_candidates \
         FROM proxy_summary_metric \
         WHERE outcome = 'measured' AND created_at >= ?1 AND (?2 IS NULL OR campaign_id = ?2) \
         GROUP BY conversation_hash ORDER BY conversation_hash",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_all(pool)
    .await?;

    let failure_denominator = agg.measured + agg.count_failures + agg.generation_failures;
    let failure_rate = if failure_denominator == 0 {
        0.0
    } else {
        (agg.count_failures + agg.generation_failures) as f64 / failure_denominator as f64
    };
    let pct_meets_low_water = if agg.measured == 0 {
        0.0
    } else {
        agg.meets_low_water_count as f64 / agg.measured as f64
    };
    let pct_meets_two_turn = if agg.measured == 0 {
        0.0
    } else {
        agg.meets_two_turn_count as f64 / agg.measured as f64
    };

    let (q_h_min, q_h_median, q_h_max, q_h_avg) = stats(&qs);
    let (n_h_min, n_h_median, n_h_max, n_h_avg) = stats(&ns);

    Ok(SummaryReport {
        total_admitted: agg.total_admitted,
        measured: agg.measured,
        disarmed: agg.disarmed,
        below_ceiling: agg.below_ceiling,
        tail_boundary_failures: agg.tail_boundary_failures,
        no_candidate: agg.no_candidate,
        count_failures: agg.count_failures,
        generation_failures: agg.generation_failures,
        projection_rejected: agg.projection_rejected,
        metric_invalid: agg.metric_invalid,
        distinct_conversations: agg.distinct_conversations,
        failure_rate,
        band_400k_500k: agg.band_400k_500k,
        pct_meets_low_water,
        pct_meets_two_turn,
        q_h_min,
        q_h_median,
        q_h_max,
        q_h_avg,
        n_h_min,
        n_h_median,
        n_h_max,
        n_h_avg,
        max_plateau_turns: agg.max_plateau_turns,
        gen_input_tokens_total: agg.gen_input_tokens_total,
        gen_cache_creation_tokens_total: agg.gen_cache_creation_tokens_total,
        gen_cache_read_tokens_total: agg.gen_cache_read_tokens_total,
        gen_output_tokens_total: agg.gen_output_tokens_total,
        by_price_version_and_model,
        conversation_pass_counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    /// Deterministic 64-bit FNV-1a of a human-readable seed. Not a real
    /// SHA-256 — just a stable, syntactically valid, per-seed-distinct value
    /// for fixtures now that `insert_terminal` validates hash-shaped columns.
    fn fake_hash_raw(seed: &str) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in seed.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// 64 lowercase hex chars (SHA-256 shape) for `conversation_hash`/
    /// `source_boundary_hash`/`summary_hash` fixtures.
    fn fake_hash(seed: &str) -> String {
        format!("{:016x}", fake_hash_raw(seed)).repeat(4)
    }

    /// 16 lowercase hex chars (ctxopt `stable_checkpoint_id` shape) for
    /// `checkpoint_id` fixtures.
    fn fake_checkpoint_id(seed: &str) -> String {
        format!("{:016x}", fake_hash_raw(seed))
    }

    fn base_row(
        campaign_id: &str,
        conversation_hash: &str,
        outcome: SummaryOutcome,
    ) -> SummaryMetricInsert {
        SummaryMetricInsert {
            created_at: chrono::Utc::now().to_rfc3339(),
            campaign_id: campaign_id.into(),
            conversation_hash: fake_hash(conversation_hash),
            model: "claude-x".into(),
            response_model: None,
            method_version: "h1-shadow-summary-v1".into(),
            prompt_version: "aggregate-tool-results-v1".into(),
            price_version: "2026-07-price-v1".into(),
            checkpoint_id: None,
            source_boundary_hash: None,
            summary_hash: None,
            earliest_changed_msg: None,
            runtime_prefix_messages: None,
            tail_start_msg: None,
            tail_target_tokens: 100_000,
            source_count: None,
            protected_result_count: None,
            a_tokens: None,
            b_tokens: None,
            c_tokens: None,
            r_tokens: None,
            s_h_tokens: None,
            q_h: None,
            summary_tokens: None,
            tombstone_tokens: None,
            projected_post_tokens: None,
            byte_est_tokens: 500_000,
            tail_count_calls: None,
            gen_input_tokens: None,
            gen_cache_creation_tokens: None,
            gen_cache_read_tokens: None,
            gen_output_tokens: None,
            standard_input_usd_per_mtok: 3.0,
            standard_cache_write_usd_per_mtok: 3.75,
            standard_cache_read_usd_per_mtok: 0.3,
            standard_output_usd_per_mtok: 15.0,
            long_input_usd_per_mtok: 6.0,
            long_cache_write_usd_per_mtok: 7.5,
            long_cache_read_usd_per_mtok: 0.6,
            long_output_usd_per_mtok: 22.5,
            long_context_threshold: 200_000,
            forward_price_tier: None,
            generation_price_tier: None,
            c_gen_usd: None,
            n_h: None,
            meets_low_water: None,
            meets_two_turn: None,
            outcome,
            error_type: None,
        }
    }

    fn measured_row(
        campaign_id: &str,
        conversation_hash: &str,
        source_boundary_hash: &str,
        q_h: f64,
        n_h: f64,
        meets_low_water: bool,
        meets_two_turn: bool,
        a_tokens: i64,
    ) -> SummaryMetricInsert {
        let mut m = base_row(campaign_id, conversation_hash, SummaryOutcome::Measured);
        m.response_model = Some("claude-x".into());
        m.checkpoint_id = Some(fake_checkpoint_id("ckpt-1"));
        m.source_boundary_hash = Some(fake_hash(source_boundary_hash));
        m.summary_hash = Some("a".repeat(64));
        m.earliest_changed_msg = Some(3);
        m.runtime_prefix_messages = Some(12);
        m.tail_start_msg = Some(20);
        m.source_count = Some(5);
        m.protected_result_count = Some(1);
        m.a_tokens = Some(a_tokens);
        m.b_tokens = Some(100_000);
        m.c_tokens = Some(90_000);
        m.r_tokens = Some(a_tokens - 90_000);
        m.s_h_tokens = Some(a_tokens - 100_000);
        m.q_h = Some(q_h);
        m.summary_tokens = Some(2_000);
        m.tombstone_tokens = Some(200);
        m.projected_post_tokens = Some(310_000);
        m.tail_count_calls = Some(4);
        m.gen_input_tokens = Some(95_000);
        m.gen_cache_creation_tokens = Some(1_000);
        m.gen_cache_read_tokens = Some(2_000);
        m.gen_output_tokens = Some(500);
        m.forward_price_tier = Some(PriceTier::Standard);
        m.generation_price_tier = Some(PriceTier::Standard);
        m.c_gen_usd = Some(1.23);
        m.n_h = Some(n_h);
        m.meets_low_water = Some(meets_low_water);
        m.meets_two_turn = Some(meets_two_turn);
        m
    }

    #[tokio::test]
    async fn insert_terminal_round_trips_a_measured_row_with_all_usage_buckets() {
        let pool = connect_in_memory().await;
        insert_terminal(
            &pool,
            measured_row("camp-1", "conv-1", "b1", 0.6, 1.5, true, true, 420_000),
        )
        .await
        .unwrap();

        let (outcome, failure_stage, error_type, response_model, forward_tier, gen_read): (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        ) = sqlx::query_as(
            "SELECT outcome, failure_stage, error_type, response_model, forward_price_tier, \
             gen_cache_read_tokens FROM proxy_summary_metric",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(outcome, "measured");
        assert_eq!(failure_stage, None);
        assert_eq!(error_type, None);
        assert_eq!(response_model.as_deref(), Some("claude-x"));
        assert_eq!(forward_tier.as_deref(), Some("standard"));
        assert_eq!(gen_read, Some(2_000));
    }

    #[tokio::test]
    async fn insert_terminal_persists_disarmed_with_all_optional_fields_null() {
        let pool = connect_in_memory().await;
        insert_terminal(
            &pool,
            base_row("camp-1", "conv-1", SummaryOutcome::Disarmed),
        )
        .await
        .unwrap();

        let (outcome, failure_stage, plateau, a_tokens): (
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
        ) = sqlx::query_as(
            "SELECT outcome, failure_stage, plateau_turns, a_tokens FROM proxy_summary_metric",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(outcome, "disarmed");
        assert_eq!(failure_stage, None);
        assert_eq!(plateau, None, "plateau_turns only applies to measured rows");
        assert_eq!(a_tokens, None, "never encode not-reached as 0");
    }

    #[tokio::test]
    async fn insert_terminal_persists_count_failure_stage_and_allowlisted_error_type() {
        let pool = connect_in_memory().await;
        let mut m = base_row(
            "camp-1",
            "conv-1",
            SummaryOutcome::CountFailure(CountFailureStage::CountB),
        );
        m.error_type = Some("rate_limit_error".into());
        insert_terminal(&pool, m).await.unwrap();

        let (outcome, stage, error_type): (String, Option<String>, Option<String>) =
            sqlx::query_as("SELECT outcome, failure_stage, error_type FROM proxy_summary_metric")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(outcome, "count_failure");
        assert_eq!(stage.as_deref(), Some("count_b"));
        assert_eq!(error_type.as_deref(), Some("rate_limit_error"));
    }

    #[tokio::test]
    async fn insert_terminal_persists_generation_failure_stage() {
        let pool = connect_in_memory().await;
        let m = base_row(
            "camp-1",
            "conv-1",
            SummaryOutcome::GenerationFailure(GenerationFailureStage::Timeout),
        );
        insert_terminal(&pool, m).await.unwrap();

        let (outcome, stage): (String, Option<String>) =
            sqlx::query_as("SELECT outcome, failure_stage FROM proxy_summary_metric")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(outcome, "generation_failure");
        assert_eq!(stage.as_deref(), Some("timeout"));
    }

    // Cross-lane contract (Mellow review finding 2): every GenerationFailureStage
    // variant must round-trip through the real migration 0024 CHECK constraint —
    // this is what would have caught the missing missing_auth/transport/
    // missing_model/missing_content buckets before Lane D hit them.
    #[tokio::test]
    async fn insert_terminal_persists_every_generation_failure_stage_variant() {
        let pool = connect_in_memory().await;
        let stages = [
            GenerationFailureStage::MissingAuth,
            GenerationFailureStage::Transport,
            GenerationFailureStage::Timeout,
            GenerationFailureStage::Redirect,
            GenerationFailureStage::Non2xx,
            GenerationFailureStage::DecodeError,
            GenerationFailureStage::MissingModel,
            GenerationFailureStage::MissingContent,
            GenerationFailureStage::NonText,
            GenerationFailureStage::EmptyText,
            GenerationFailureStage::MissingUsage,
        ];
        for (i, stage) in stages.iter().enumerate() {
            insert_terminal(
                &pool,
                base_row(
                    "camp-1",
                    &format!("conv-{i}"),
                    SummaryOutcome::GenerationFailure(*stage),
                ),
            )
            .await
            .unwrap_or_else(|e| panic!("stage {stage:?} must be accepted by the CHECK: {e}"));
        }

        let persisted: Vec<Option<String>> =
            sqlx::query_scalar("SELECT failure_stage FROM proxy_summary_metric ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        let expected: Vec<Option<String>> = stages
            .iter()
            .map(|s| Some(s.as_str().to_string()))
            .collect();
        assert_eq!(persisted, expected);
    }

    #[tokio::test]
    async fn insert_terminal_persists_no_candidate_and_projection_rejected_stage_labels_only() {
        let pool = connect_in_memory().await;
        insert_terminal(
            &pool,
            base_row(
                "camp-1",
                "conv-1",
                SummaryOutcome::NoCandidate(PlanRejectStage::StructuralValidation),
            ),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            base_row(
                "camp-1",
                "conv-2",
                SummaryOutcome::ProjectionRejected(PlanRejectStage::ReplacementDidNotShrink),
            ),
        )
        .await
        .unwrap();

        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT outcome, failure_stage FROM proxy_summary_metric ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            rows[0],
            ("no_candidate".into(), Some("structural_validation".into()))
        );
        assert_eq!(
            rows[1],
            (
                "projection_rejected".into(),
                Some("replacement_did_not_shrink".into())
            )
        );
    }

    #[tokio::test]
    async fn insert_terminal_persists_metric_invalid_stage() {
        let pool = connect_in_memory().await;
        insert_terminal(
            &pool,
            base_row(
                "camp-1",
                "conv-1",
                SummaryOutcome::MetricInvalid(MetricInvalidStage::SHGreaterThanR),
            ),
        )
        .await
        .unwrap();

        let stage: Option<String> =
            sqlx::query_scalar("SELECT failure_stage FROM proxy_summary_metric")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stage.as_deref(), Some("s_h_gt_r"));
    }

    #[tokio::test]
    async fn insert_terminal_rejects_error_type_outside_the_allowlist_and_persists_nothing() {
        let pool = connect_in_memory().await;
        let mut hostile = base_row(
            "camp-1",
            "conv-1",
            SummaryOutcome::CountFailure(CountFailureStage::CountA),
        );
        hostile.error_type = Some(
            "IGNORE ALL PREVIOUS INSTRUCTIONS; leaked-key sk-live-hostile <<<SOURCE>>>".into(),
        );
        let err = insert_terminal(&pool, hostile).await;
        assert!(
            err.is_err(),
            "an out-of-allowlist error_type must be rejected"
        );

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM proxy_summary_metric")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count, 0,
            "the rejected insert must not commit a partial row"
        );
    }

    // PRIVACY (Mellow review finding 1, HIGH): unlike error_type/outcome/
    // failure_stage, the hash-shaped columns (conversation_hash,
    // source_boundary_hash, summary_hash, checkpoint_id) had NO shape
    // validation — insert_terminal accepted raw text verbatim in place of a
    // hash. This is the literal hostile-mock-content scan the plan's
    // Persistence section asks for: every hash-shaped field must reject raw
    // content, and nothing hostile may reach a committed row.
    #[tokio::test]
    async fn insert_terminal_rejects_hostile_raw_content_in_hash_shaped_fields_and_persists_nothing(
    ) {
        let pool = connect_in_memory().await;
        const HOSTILE: &str =
            "RAW SUMMARY TEXT dumped verbatim <<<SOURCE-MARKER>>> api key sk-live-hostile-secret";

        let mut bad_conversation =
            measured_row("camp-1", "conv-1", "b1", 0.5, 1.0, true, true, 420_000);
        bad_conversation.conversation_hash = HOSTILE.into();
        assert!(
            insert_terminal(&pool, bad_conversation).await.is_err(),
            "a non-hex conversation_hash must be rejected"
        );

        let mut bad_summary = measured_row("camp-1", "conv-1", "b1", 0.5, 1.0, true, true, 420_000);
        bad_summary.summary_hash = Some(HOSTILE.into());
        assert!(
            insert_terminal(&pool, bad_summary).await.is_err(),
            "a non-hex summary_hash must be rejected"
        );

        let mut bad_boundary =
            measured_row("camp-1", "conv-1", "b1", 0.5, 1.0, true, true, 420_000);
        bad_boundary.source_boundary_hash = Some(HOSTILE.into());
        assert!(
            insert_terminal(&pool, bad_boundary).await.is_err(),
            "a non-hex source_boundary_hash must be rejected"
        );

        let mut bad_checkpoint =
            measured_row("camp-1", "conv-1", "b1", 0.5, 1.0, true, true, 420_000);
        bad_checkpoint.checkpoint_id = Some(HOSTILE.into());
        assert!(
            insert_terminal(&pool, bad_checkpoint).await.is_err(),
            "a non-hex checkpoint_id must be rejected"
        );

        // A legitimate row (real-shaped hashes) still succeeds.
        insert_terminal(
            &pool,
            measured_row("camp-1", "conv-1", "b1", 0.5, 1.0, true, true, 420_000),
        )
        .await
        .unwrap();

        // Scan every text column of every committed row: the hostile content
        // must never appear anywhere, and only the one legitimate row exists.
        let rows: Vec<(
            String,
            String,
            String,
            Option<String>,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        )> = sqlx::query_as(
            "SELECT campaign_id, conversation_hash, model, response_model, method_version, \
             prompt_version, price_version, checkpoint_id, source_boundary_hash, summary_hash \
             FROM proxy_summary_metric",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 1, "only the one legitimate row must commit");
        let dump = format!("{rows:?}");
        assert!(
            !dump.contains("RAW SUMMARY TEXT"),
            "hostile content leaked into a committed row: {dump}"
        );
        assert!(
            !dump.contains("sk-live-hostile-secret"),
            "a credential-shaped marker leaked into a committed row: {dump}"
        );
        assert!(
            !dump.contains("SOURCE-MARKER"),
            "a source marker leaked into a committed row: {dump}"
        );
    }

    #[tokio::test]
    async fn insert_terminal_measured_row_without_source_boundary_hash_is_rejected() {
        let pool = connect_in_memory().await;
        let mut m = measured_row("camp-1", "conv-1", "b1", 0.5, 1.0, true, true, 420_000);
        m.source_boundary_hash = None;
        let err = insert_terminal(&pool, m).await;
        assert!(
            err.is_err(),
            "a measured row cannot derive plateau_turns without source_boundary_hash"
        );
    }

    #[tokio::test]
    async fn plateau_turns_increments_on_same_boundary_and_resets_on_change() {
        let pool = connect_in_memory().await;

        insert_terminal(
            &pool,
            measured_row("camp-1", "conv-1", "b1", 0.5, 1.0, true, true, 420_000),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            measured_row("camp-1", "conv-1", "b1", 0.5, 1.0, true, true, 420_000),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            measured_row("camp-1", "conv-1", "b2", 0.5, 1.0, true, true, 420_000),
        )
        .await
        .unwrap();
        // Different conversation with the same boundary starts fresh.
        insert_terminal(
            &pool,
            measured_row("camp-1", "conv-2", "b1", 0.5, 1.0, true, true, 420_000),
        )
        .await
        .unwrap();
        // Different campaign with the same conversation/boundary starts fresh.
        insert_terminal(
            &pool,
            measured_row("camp-2", "conv-1", "b1", 0.5, 1.0, true, true, 420_000),
        )
        .await
        .unwrap();

        let plateaus: Vec<i64> = sqlx::query_scalar(
            "SELECT plateau_turns FROM proxy_summary_metric \
             WHERE campaign_id = 'camp-1' AND conversation_hash = ?1 ORDER BY id",
        )
        .bind(fake_hash("conv-1"))
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(plateaus, vec![0, 1, 0]);

        let other_conv: i64 = sqlx::query_scalar(
            "SELECT plateau_turns FROM proxy_summary_metric \
             WHERE campaign_id = 'camp-1' AND conversation_hash = ?1",
        )
        .bind(fake_hash("conv-2"))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(other_conv, 0);

        let other_campaign: i64 = sqlx::query_scalar(
            "SELECT plateau_turns FROM proxy_summary_metric WHERE campaign_id = 'camp-2'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(other_campaign, 0);
    }

    #[tokio::test]
    async fn report_computes_pooled_metrics_and_never_hides_an_all_miss_conversation() {
        let pool = connect_in_memory().await;

        // conv-pass: two measured rows that both pass both bars.
        insert_terminal(
            &pool,
            measured_row("camp-1", "conv-pass", "b1", 0.8, 1.0, true, true, 450_000),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            measured_row("camp-1", "conv-pass", "b1", 0.9, 1.2, true, true, 420_000),
        )
        .await
        .unwrap();
        // conv-all-miss: measured, but misses both bars every time — a pooled
        // percentage across all rows could stay high while this conversation
        // never once passes.
        insert_terminal(
            &pool,
            measured_row(
                "camp-1",
                "conv-all-miss",
                "b2",
                0.1,
                5.0,
                false,
                false,
                480_000,
            ),
        )
        .await
        .unwrap();
        // Non-measured admissions: must not dilute failure_rate or pass stats.
        insert_terminal(
            &pool,
            base_row("camp-1", "conv-below", SummaryOutcome::BelowCeiling),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            base_row(
                "camp-1",
                "conv-cf",
                SummaryOutcome::CountFailure(CountFailureStage::CountA),
            ),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            base_row(
                "camp-1",
                "conv-gf",
                SummaryOutcome::GenerationFailure(GenerationFailureStage::Timeout),
            ),
        )
        .await
        .unwrap();

        let r = report(&pool, 24, Some("camp-1")).await.unwrap();
        assert_eq!(r.total_admitted, 6);
        assert_eq!(r.measured, 3);
        assert_eq!(r.below_ceiling, 1);
        assert_eq!(r.count_failures, 1);
        assert_eq!(r.generation_failures, 1);
        assert_eq!(
            r.distinct_conversations, 2,
            "only conv-pass and conv-all-miss are measured"
        );
        // failure_rate denominator is measured + count_failure + generation_failure = 5.
        assert!((r.failure_rate - (2.0 / 5.0)).abs() < 1e-9);
        assert_eq!(
            r.band_400k_500k, 3,
            "all three measured rows have a_tokens in [400k,500k]"
        );
        // pct bars: 2 of 3 measured rows pass both.
        assert!((r.pct_meets_low_water - (2.0 / 3.0)).abs() < 1e-9);
        assert!((r.pct_meets_two_turn - (2.0 / 3.0)).abs() < 1e-9);

        let all_miss = r
            .conversation_pass_counts
            .iter()
            .find(|c| c.conversation_hash == fake_hash("conv-all-miss"))
            .expect("conv-all-miss must appear in the per-conversation distribution");
        assert_eq!(all_miss.measured_candidates, 1);
        assert_eq!(
            all_miss.passing_candidates, 0,
            "an all-miss conversation must show zero passes, not be hidden by the pooled %"
        );

        let pass = r
            .conversation_pass_counts
            .iter()
            .find(|c| c.conversation_hash == fake_hash("conv-pass"))
            .unwrap();
        assert_eq!(pass.measured_candidates, 2);
        assert_eq!(pass.passing_candidates, 2);
    }

    #[tokio::test]
    async fn report_scopes_by_campaign_id_when_provided() {
        let pool = connect_in_memory().await;
        insert_terminal(
            &pool,
            measured_row("camp-a", "conv-1", "b1", 0.5, 1.0, true, true, 420_000),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            measured_row("camp-b", "conv-2", "b1", 0.5, 1.0, true, true, 420_000),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            measured_row("camp-b", "conv-3", "b1", 0.5, 1.0, true, true, 420_000),
        )
        .await
        .unwrap();

        let scoped = report(&pool, 24, Some("camp-a")).await.unwrap();
        assert_eq!(scoped.measured, 1);

        let all = report(&pool, 24, None).await.unwrap();
        assert_eq!(all.measured, 3);
    }

    #[tokio::test]
    async fn report_on_empty_table_is_all_zero() {
        let pool = connect_in_memory().await;
        let r = report(&pool, 24, None).await.unwrap();
        assert_eq!(r.total_admitted, 0);
        assert_eq!(r.measured, 0);
        assert_eq!(r.failure_rate, 0.0);
        assert_eq!(r.pct_meets_low_water, 0.0);
        assert_eq!(r.pct_meets_two_turn, 0.0);
        assert_eq!(r.q_h_min, 0.0);
        assert_eq!(r.n_h_max, 0.0);
        assert!(r.by_price_version_and_model.is_empty());
        assert!(r.conversation_pass_counts.is_empty());
    }

    #[tokio::test]
    async fn report_groups_by_price_version_and_model() {
        let pool = connect_in_memory().await;
        let mut a = measured_row("camp-1", "conv-1", "b1", 0.5, 1.0, true, true, 420_000);
        a.price_version = "price-v1".into();
        a.model = "claude-x".into();
        insert_terminal(&pool, a).await.unwrap();

        let mut b = measured_row("camp-1", "conv-2", "b1", 0.5, 1.0, true, true, 420_000);
        b.price_version = "price-v2".into();
        b.model = "claude-y".into();
        insert_terminal(&pool, b).await.unwrap();

        let r = report(&pool, 24, None).await.unwrap();
        assert_eq!(r.by_price_version_and_model.len(), 2);
        assert!(r
            .by_price_version_and_model
            .iter()
            .any(|g| g.price_version == "price-v1" && g.model == "claude-x" && g.count == 1));
        assert!(r
            .by_price_version_and_model
            .iter()
            .any(|g| g.price_version == "price-v2" && g.model == "claude-y" && g.count == 1));
    }

    // PRIVACY (Global constraint #7): the schema must not gain a column
    // shaped to hold a request body, source text, summary string, prompt, or
    // credential. This is a positive-control structural guard — it fails the
    // moment anyone adds such a column, the same way
    // `migration_files_are_contiguous_and_migrate_reaches_the_max` guards
    // migration numbering.
    #[tokio::test]
    async fn schema_column_list_matches_the_bounded_label_and_hash_contract() {
        let pool = connect_in_memory().await;
        let mut columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('proxy_summary_metric')")
                .fetch_all(&pool)
                .await
                .unwrap();
        columns.sort();

        let mut expected: Vec<&str> = vec![
            "id",
            "created_at",
            "campaign_id",
            "conversation_hash",
            "model",
            "response_model",
            "method_version",
            "prompt_version",
            "price_version",
            "checkpoint_id",
            "source_boundary_hash",
            "summary_hash",
            "earliest_changed_msg",
            "runtime_prefix_messages",
            "tail_start_msg",
            "tail_target_tokens",
            "source_count",
            "protected_result_count",
            "a_tokens",
            "b_tokens",
            "c_tokens",
            "r_tokens",
            "s_h_tokens",
            "q_h",
            "summary_tokens",
            "tombstone_tokens",
            "projected_post_tokens",
            "byte_est_tokens",
            "tail_count_calls",
            "plateau_turns",
            "gen_input_tokens",
            "gen_cache_creation_tokens",
            "gen_cache_read_tokens",
            "gen_output_tokens",
            "standard_input_usd_per_mtok",
            "standard_cache_write_usd_per_mtok",
            "standard_cache_read_usd_per_mtok",
            "standard_output_usd_per_mtok",
            "long_input_usd_per_mtok",
            "long_cache_write_usd_per_mtok",
            "long_cache_read_usd_per_mtok",
            "long_output_usd_per_mtok",
            "long_context_threshold",
            "forward_price_tier",
            "generation_price_tier",
            "c_gen_usd",
            "n_h",
            "meets_low_water",
            "meets_two_turn",
            "outcome",
            "failure_stage",
            "error_type",
        ];
        expected.sort_unstable();
        assert_eq!(
            columns, expected,
            "schema changed — re-verify no new column can carry a body/summary/credential"
        );
    }

    // PRIVACY: even bypassing this module's Rust-level enum entirely and
    // inserting raw SQL (as a hostile or buggy caller might), the DB CHECK
    // constraints are the final backstop rejecting a forged outcome, stage,
    // or error_type — so no such value, and nothing an attacker smuggled
    // inside it, can ever land in a committed row.
    #[tokio::test]
    async fn check_constraints_reject_forged_outcome_stage_and_error_type_at_the_db_layer() {
        let pool = connect_in_memory().await;

        let bogus_outcome = sqlx::query(
            "INSERT INTO proxy_summary_metric \
             (created_at, campaign_id, conversation_hash, model, method_version, \
              prompt_version, price_version, tail_target_tokens, byte_est_tokens, \
              standard_input_usd_per_mtok, standard_cache_write_usd_per_mtok, \
              standard_cache_read_usd_per_mtok, standard_output_usd_per_mtok, \
              long_input_usd_per_mtok, long_cache_write_usd_per_mtok, \
              long_cache_read_usd_per_mtok, long_output_usd_per_mtok, \
              long_context_threshold, outcome) \
             VALUES ('2026-01-01T00:00:00Z', 'c', 'h', 'm', 'mv', 'pv', 'pricev', 100000, \
              500000, 1, 1, 1, 1, 1, 1, 1, 1, 200000, \
              '<script>alert(1)</script>')",
        )
        .execute(&pool)
        .await;
        assert!(
            bogus_outcome.is_err(),
            "an unknown outcome must violate the CHECK"
        );

        let bogus_stage = sqlx::query(
            "INSERT INTO proxy_summary_metric \
             (created_at, campaign_id, conversation_hash, model, method_version, \
              prompt_version, price_version, tail_target_tokens, byte_est_tokens, \
              standard_input_usd_per_mtok, standard_cache_write_usd_per_mtok, \
              standard_cache_read_usd_per_mtok, standard_output_usd_per_mtok, \
              long_input_usd_per_mtok, long_cache_write_usd_per_mtok, \
              long_cache_read_usd_per_mtok, long_output_usd_per_mtok, \
              long_context_threshold, outcome, failure_stage) \
             VALUES ('2026-01-01T00:00:00Z', 'c', 'h', 'm', 'mv', 'pv', 'pricev', 100000, \
              500000, 1, 1, 1, 1, 1, 1, 1, 1, 200000, \
              'count_failure', 'DROP TABLE proxy_summary_metric;--')",
        )
        .execute(&pool)
        .await;
        assert!(
            bogus_stage.is_err(),
            "an unknown failure_stage must violate the CHECK"
        );

        let bogus_error_type = sqlx::query(
            "INSERT INTO proxy_summary_metric \
             (created_at, campaign_id, conversation_hash, model, method_version, \
              prompt_version, price_version, tail_target_tokens, byte_est_tokens, \
              standard_input_usd_per_mtok, standard_cache_write_usd_per_mtok, \
              standard_cache_read_usd_per_mtok, standard_output_usd_per_mtok, \
              long_input_usd_per_mtok, long_cache_write_usd_per_mtok, \
              long_cache_read_usd_per_mtok, long_output_usd_per_mtok, \
              long_context_threshold, outcome, failure_stage, error_type) \
             VALUES ('2026-01-01T00:00:00Z', 'c', 'h', 'm', 'mv', 'pv', 'pricev', 100000, \
              500000, 1, 1, 1, 1, 1, 1, 1, 1, 200000, \
              'count_failure', 'count_a', 'leaked upstream body: sk-live-hostile')",
        )
        .execute(&pool)
        .await;
        assert!(
            bogus_error_type.is_err(),
            "an unknown error_type must violate the CHECK"
        );

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM proxy_summary_metric")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count, 0,
            "every forged attempt must be rejected, not partially committed"
        );
    }
}

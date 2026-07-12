//! H2 shadow-quality metric contract (plan
//! `docs/superpowers/plans/2026-07-12-hybrid-h2-shadow-quality.md`, section
//! "Persistence: migration 0025 and repository").
//!
//! One terminal row per reserved quality case (Global constraint #9), plus the
//! bounded human-audit verdict table. Global constraint #8 (Privacy): counts,
//! bounded enums, booleans, model/prompt versions, usage totals, and SHA-256
//! hashes only — never a probe, plan, summary, judge rationale, credential, or
//! raw upstream error. The evaluator preflight's credential/upstream identity
//! hash is in-memory only and deliberately has no column or field anywhere in
//! this module.
//!
//! [`insert_terminal`] is the sole metric write API and accepts only typed
//! outcome/stage/kind enums — no `String` payload can ride a failure.
//! [`insert_audit`] is the sole audit write API: fixture-only, one-shot
//! (`UNIQUE(quality_metric_id)`), with no update API at all — re-judging means
//! a new campaign, never an edited row.
//!
//! The failure vocabulary is pinned to Lane B's ACTUAL merged
//! `QualityClientError` (`runtime/quality.rs`: `QualityCallStage::as_str`,
//! every `new(stage, ...)` kind literal, and `error_type` populated only for
//! `non_2xx` from the eight allowlisted Anthropic types) — read from the code,
//! not the plan prose (the H1 allowlist-from-prose lesson, ruling on
//! hybrid-h1-lane-b-metric).

use serde::Serialize;
use sqlx::SqlitePool;

use crate::engine::runtime::quality::{
    paired_cluster_bootstrap, BehaviorCase, BootstrapError, QualityTag,
    FAITHFULNESS_PROMPT_VERSION, JUDGE_PROMPT_VERSION, PROBE_PROMPT_VERSION,
    QUALITY_METHOD_VERSION, QUALITY_RUBRIC_VERSION, REPLAY_PROMPT_VERSION,
};

/// The eight allowlisted Anthropic `error.type` values
/// (`count_tokens::KNOWN_ERROR_TYPES`), typed so a failure physically cannot
/// carry an unlisted value. Lane B populates this only for `non_2xx`; unknown
/// or unparsed upstream types are dropped to `None` before they reach here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamErrorType {
    InvalidRequestError,
    AuthenticationError,
    PermissionError,
    NotFoundError,
    RequestTooLarge,
    RateLimitError,
    ApiError,
    OverloadedError,
}

impl UpstreamErrorType {
    pub const ALL: [Self; 8] = [
        Self::InvalidRequestError,
        Self::AuthenticationError,
        Self::PermissionError,
        Self::NotFoundError,
        Self::RequestTooLarge,
        Self::RateLimitError,
        Self::ApiError,
        Self::OverloadedError,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequestError => "invalid_request_error",
            Self::AuthenticationError => "authentication_error",
            Self::PermissionError => "permission_error",
            Self::NotFoundError => "not_found_error",
            Self::RequestTooLarge => "request_too_large",
            Self::RateLimitError => "rate_limit_error",
            Self::ApiError => "api_error",
            Self::OverloadedError => "overloaded_error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == value)
    }
}

/// Lane B's 18 bounded `QualityClientError::kind()` labels, verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityFailureKind {
    MissingAuth,
    Timeout,
    Transport,
    Redirect,
    Non2xx,
    Decode,
    MissingModel,
    MissingContent,
    NonTextContent,
    EmptyText,
    MissingUsage,
    SchemaJson,
    SchemaShape,
    SchemaBounds,
    SchemaReconcile,
    SchemaCitation,
    SchemaDuplicate,
    SchemaCategory,
}

impl QualityFailureKind {
    pub const ALL: [Self; 18] = [
        Self::MissingAuth,
        Self::Timeout,
        Self::Transport,
        Self::Redirect,
        Self::Non2xx,
        Self::Decode,
        Self::MissingModel,
        Self::MissingContent,
        Self::NonTextContent,
        Self::EmptyText,
        Self::MissingUsage,
        Self::SchemaJson,
        Self::SchemaShape,
        Self::SchemaBounds,
        Self::SchemaReconcile,
        Self::SchemaCitation,
        Self::SchemaDuplicate,
        Self::SchemaCategory,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingAuth => "missing_auth",
            Self::Timeout => "timeout",
            Self::Transport => "transport",
            Self::Redirect => "redirect",
            Self::Non2xx => "non_2xx",
            Self::Decode => "decode",
            Self::MissingModel => "missing_model",
            Self::MissingContent => "missing_content",
            Self::NonTextContent => "non_text_content",
            Self::EmptyText => "empty_text",
            Self::MissingUsage => "missing_usage",
            Self::SchemaJson => "schema_json",
            Self::SchemaShape => "schema_shape",
            Self::SchemaBounds => "schema_bounds",
            Self::SchemaReconcile => "schema_reconcile",
            Self::SchemaCitation => "schema_citation",
            Self::SchemaDuplicate => "schema_duplicate",
            Self::SchemaCategory => "schema_category",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == value)
    }
}

/// The five in-case quality calls (Lane B `QualityCallStage` minus
/// `preflight`, which is its own outcome so a `call_failure` row can never
/// masquerade as a preflight and vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityCaseStage {
    Probe,
    Faithfulness,
    OriginalReplay,
    ProjectedReplay,
    Judge,
}

impl QualityCaseStage {
    pub const ALL: [Self; 5] = [
        Self::Probe,
        Self::Faithfulness,
        Self::OriginalReplay,
        Self::ProjectedReplay,
        Self::Judge,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Faithfulness => "faithfulness",
            Self::OriginalReplay => "original_replay",
            Self::ProjectedReplay => "projected_replay",
            Self::Judge => "judge",
        }
    }
}

/// A bounded call failure: which call, Lane B's kind, and — only for
/// `Non2xx` — the allowlisted upstream `error.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualityCallFailure {
    pub kind: QualityFailureKind,
    pub error_type: Option<UpstreamErrorType>,
}

/// The row's single terminal state. Exactly one variant per
/// [`insert_terminal`] call; `UNIQUE(case_id)` additionally makes a second
/// terminal row for the same reserved case a hard DB error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityOutcome {
    /// The campaign's evaluator-auth preflight succeeded (consumes the first
    /// reserved case; plan §"Evaluator authentication preflight").
    PreflightOk,
    /// The preflight failed; H2 auto-disarms. `failure_stage` is `preflight`.
    PreflightFailure(QualityCallFailure),
    /// H0 projection revalidation failed before any quality call — a hard H2
    /// NO-GO signal.
    StructuralFailure,
    /// One of the five case calls failed (call/schema); the case is consumed.
    CallFailure(QualityCaseStage, QualityCallFailure),
    /// The H2 epoch was lost between calls; the case is consumed with zero
    /// further calls.
    Disarmed,
    /// All five calls completed and parsed; every measurement column is set.
    Completed,
}

impl QualityOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::PreflightOk => "preflight_ok",
            Self::PreflightFailure(_) => "preflight_failure",
            Self::StructuralFailure => "structural_failure",
            Self::CallFailure(_, _) => "call_failure",
            Self::Disarmed => "disarmed",
            Self::Completed => "completed",
        }
    }

    fn stage_label(self) -> Option<&'static str> {
        match self {
            Self::PreflightFailure(_) => Some("preflight"),
            Self::CallFailure(stage, _) => Some(stage.as_str()),
            _ => None,
        }
    }

    fn failure(self) -> Option<QualityCallFailure> {
        match self {
            Self::PreflightFailure(f) | Self::CallFailure(_, f) => Some(f),
            _ => None,
        }
    }
}

/// Where the case came from. Fixture identity is present exactly when the
/// case is a fixture case — by construction, mirrored by the DB CHECKs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualityCaseSource {
    Live,
    Fixture { id: String, family: String },
}

/// Blind-judge comparison label (plan call 5). Derivable from the two overall
/// passes; [`insert_terminal`] rejects an inconsistent label so "a tie of two
/// failures" can never be recorded as anything but `BothFail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonLabel {
    OriginalWin,
    Tie,
    ProjectedWin,
    BothFail,
}

impl ComparisonLabel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OriginalWin => "original_win",
            Self::Tie => "tie",
            Self::ProjectedWin => "projected_win",
            Self::BothFail => "both_fail",
        }
    }

    fn expected(original_pass: bool, projected_pass: bool) -> Self {
        match (original_pass, projected_pass) {
            (true, false) => Self::OriginalWin,
            (true, true) => Self::Tie,
            (false, true) => Self::ProjectedWin,
            (false, false) => Self::BothFail,
        }
    }
}

/// Provider usage for one role call (all four buckets, per ruling `0807943e`'s
/// no-cache-hit-claim discipline: actual buckets, never an inferred hit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleUsage {
    pub input_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
}

/// Three blind-judge dimension booleans plus the overall pass for one plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanVerdict {
    pub correct: bool,
    pub constraint_adherent: bool,
    pub next_action_match: bool,
    pub pass: bool,
}

/// One terminal H2 case. Version columns are deliberately NOT fields: the
/// insert writes the compiled Lane A constants directly, so a row always
/// names exactly the code that produced it and can never disagree with the
/// armed configuration's pinned versions.
pub struct QualityMetricInsert {
    // identity
    pub created_at: String,
    pub quality_campaign_id: String,
    pub h1_campaign_id: String,
    pub case_id: String,
    pub source: QualityCaseSource,
    pub tags: Vec<QualityTag>,
    pub conversation_hash: String,
    pub checkpoint_id: Option<String>,
    pub source_boundary_hash: Option<String>,
    pub summary_hash: Option<String>,
    pub probe_set_hash: Option<String>,
    pub original_plan_hash: Option<String>,
    pub projected_plan_hash: Option<String>,
    pub judge_hash: Option<String>,

    // models
    pub task_model: String,
    pub summarizer_response_model: Option<String>,
    pub evaluator_response_model: Option<String>,

    // structure/faithfulness
    pub structural_pass: Option<bool>,
    pub claims_total: Option<i64>,
    pub claims_supported: Option<i64>,
    pub claims_unsupported: Option<i64>,
    pub critical_hallucinations: Option<i64>,
    pub noncritical_hallucinations: Option<i64>,
    pub critical_omissions: Option<i64>,
    pub noncritical_omissions: Option<i64>,
    pub probes_total: Option<i64>,
    pub probes_retained: Option<i64>,
    pub probe_recall: Option<f64>,

    // behavior
    pub original: Option<PlanVerdict>,
    pub projected: Option<PlanVerdict>,
    pub comparison: Option<ComparisonLabel>,

    // usage
    pub preflight_usage: Option<RoleUsage>,
    pub probe_usage: Option<RoleUsage>,
    pub faithfulness_usage: Option<RoleUsage>,
    pub original_replay_usage: Option<RoleUsage>,
    pub projected_replay_usage: Option<RoleUsage>,
    pub judge_usage: Option<RoleUsage>,

    // terminal
    pub outcome: QualityOutcome,
}

fn is_lowercase_hex_of_len(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Lowercase hyphenated UUID (8-4-4-4-12): the exact shape of
/// `uuid::Uuid::new_v4().to_string()`, which mints every campaign/case id.
fn is_lowercase_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => matches!(b, b'0'..=b'9' | b'a'..=b'f'),
        })
}

fn protocol(msg: impl Into<String>) -> sqlx::Error {
    sqlx::Error::Protocol(format!("[proxy_quality_metric] {}", msg.into()))
}

fn require_uuid(value: &str, field: &str) -> sqlx::Result<()> {
    if is_lowercase_uuid(value) {
        Ok(())
    } else {
        Err(protocol(format!(
            "{field} must be a lowercase hyphenated UUID"
        )))
    }
}

fn require_sha256(value: Option<&str>, field: &str) -> sqlx::Result<()> {
    match value {
        None => Ok(()),
        Some(v) if is_lowercase_hex_of_len(v, 64) => Ok(()),
        Some(_) => Err(protocol(format!(
            "{field} must be a 64-char lowercase SHA-256 hex digest"
        ))),
    }
}

fn validate(m: &QualityMetricInsert) -> sqlx::Result<()> {
    require_uuid(&m.quality_campaign_id, "quality_campaign_id")?;
    require_uuid(&m.h1_campaign_id, "h1_campaign_id")?;
    require_uuid(&m.case_id, "case_id")?;
    if !is_lowercase_hex_of_len(&m.conversation_hash, 64) {
        return Err(protocol(
            "conversation_hash must be a 64-char lowercase SHA-256 hex digest",
        ));
    }
    if let Some(id) = m.checkpoint_id.as_deref() {
        if !is_lowercase_hex_of_len(id, 16) {
            return Err(protocol(
                "checkpoint_id must be a 16-char lowercase hex identity key",
            ));
        }
    }
    require_sha256(m.source_boundary_hash.as_deref(), "source_boundary_hash")?;
    require_sha256(m.summary_hash.as_deref(), "summary_hash")?;
    require_sha256(m.probe_set_hash.as_deref(), "probe_set_hash")?;
    require_sha256(m.original_plan_hash.as_deref(), "original_plan_hash")?;
    require_sha256(m.projected_plan_hash.as_deref(), "projected_plan_hash")?;
    require_sha256(m.judge_hash.as_deref(), "judge_hash")?;

    if let Some(failure) = m.outcome.failure() {
        if failure.error_type.is_some() && failure.kind != QualityFailureKind::Non2xx {
            return Err(protocol(
                "error_type may only accompany the non_2xx failure kind",
            ));
        }
    }

    match m.outcome {
        QualityOutcome::StructuralFailure => {
            if m.structural_pass != Some(false) {
                return Err(protocol(
                    "a structural_failure row must record structural_pass = false",
                ));
            }
        }
        QualityOutcome::Completed => {
            if m.structural_pass != Some(true) {
                return Err(protocol(
                    "a completed row must record structural_pass = true",
                ));
            }
            let (total, supported, unsupported) =
                match (m.claims_total, m.claims_supported, m.claims_unsupported) {
                    (Some(t), Some(s), Some(u)) => (t, s, u),
                    _ => return Err(protocol("a completed row requires all claim counts")),
                };
            if supported + unsupported != total {
                return Err(protocol(
                    "claims_supported + claims_unsupported must equal claims_total",
                ));
            }
            let (probes_total, probes_retained) = match (m.probes_total, m.probes_retained) {
                (Some(t), Some(r)) => (t, r),
                _ => return Err(protocol("a completed row requires probe counts")),
            };
            if probes_retained > probes_total || probes_total <= 0 {
                return Err(protocol(
                    "probes_retained must be <= probes_total and probes_total > 0",
                ));
            }
            let recall = m
                .probe_recall
                .ok_or_else(|| protocol("a completed row requires probe_recall"))?;
            let expected = probes_retained as f64 / probes_total as f64;
            if (recall - expected).abs() > 1e-9 {
                return Err(protocol(
                    "probe_recall must equal probes_retained / probes_total",
                ));
            }
            if m.critical_hallucinations.is_none()
                || m.noncritical_hallucinations.is_none()
                || m.critical_omissions.is_none()
                || m.noncritical_omissions.is_none()
            {
                return Err(protocol(
                    "a completed row requires hallucination and omission counts",
                ));
            }
            let (original, projected) = match (m.original, m.projected) {
                (Some(o), Some(p)) => (o, p),
                _ => return Err(protocol("a completed row requires both plan verdicts")),
            };
            // Overall pass IS the conjunction of the three dimensions (plan
            // call 5: "Overall pass is all three true") — both directions, so
            // a forged pass can't inflate the GO pass-rate columns and a
            // forged fail can't deflate them (Mellow review, ruling on
            // hybrid-h2-lane-c).
            for (verdict, side) in [(original, "original"), (projected, "projected")] {
                let expected =
                    verdict.correct && verdict.constraint_adherent && verdict.next_action_match;
                if verdict.pass != expected {
                    return Err(protocol(format!(
                        "{side} overall pass is inconsistent with its three dimension booleans"
                    )));
                }
            }
            let comparison = m
                .comparison
                .ok_or_else(|| protocol("a completed row requires a comparison label"))?;
            if comparison != ComparisonLabel::expected(original.pass, projected.pass) {
                return Err(protocol(
                    "comparison label is inconsistent with the recorded overall passes",
                ));
            }
        }
        _ => {}
    }

    Ok(())
}

/// Insert one terminal quality row. The sole metric write API: typed
/// outcome/stage/kind enums only, fail-closed hash/UUID shape validation,
/// per-outcome invariants, and version columns written from the compiled
/// Lane A constants.
pub async fn insert_terminal(pool: &SqlitePool, m: QualityMetricInsert) -> sqlx::Result<()> {
    validate(&m)?;

    let (source_kind, fixture_id, fixture_family) = match &m.source {
        QualityCaseSource::Live => ("live", None, None),
        QualityCaseSource::Fixture { id, family } => {
            ("fixture", Some(id.clone()), Some(family.clone()))
        }
    };
    let tag = |t: QualityTag| i64::from(m.tags.contains(&t));
    let usage = |u: Option<RoleUsage>| {
        (
            u.map(|u| u.input_tokens),
            u.map(|u| u.cache_creation_tokens),
            u.map(|u| u.cache_read_tokens),
            u.map(|u| u.output_tokens),
        )
    };
    let (pf_in, pf_cc, pf_cr, pf_out) = usage(m.preflight_usage);
    let (pr_in, pr_cc, pr_cr, pr_out) = usage(m.probe_usage);
    let (fa_in, fa_cc, fa_cr, fa_out) = usage(m.faithfulness_usage);
    let (or_in, or_cc, or_cr, or_out) = usage(m.original_replay_usage);
    let (pj_in, pj_cc, pj_cr, pj_out) = usage(m.projected_replay_usage);
    let (ju_in, ju_cc, ju_cr, ju_out) = usage(m.judge_usage);

    sqlx::query(
        "INSERT INTO proxy_quality_metric (\
         created_at, quality_campaign_id, h1_campaign_id, case_id, source_kind, \
         fixture_id, fixture_family, \
         tag_side_effecting_output, tag_long_log, tag_exact_error, tag_rejected_alternative, \
         tag_parallel_tool_cycle, tag_prompt_like_tool_text, tag_mutation_or_open_work, \
         conversation_hash, checkpoint_id, source_boundary_hash, summary_hash, \
         probe_set_hash, original_plan_hash, projected_plan_hash, judge_hash, \
         task_model, summarizer_response_model, evaluator_response_model, \
         method_version, rubric_version, probe_prompt_version, faithfulness_prompt_version, \
         replay_prompt_version, judge_prompt_version, \
         structural_pass, claims_total, claims_supported, claims_unsupported, \
         critical_hallucinations, noncritical_hallucinations, critical_omissions, \
         noncritical_omissions, probes_total, probes_retained, probe_recall, \
         original_correct, original_constraint_adherent, original_next_action_match, \
         projected_correct, projected_constraint_adherent, projected_next_action_match, \
         original_pass, projected_pass, comparison, \
         preflight_input_tokens, preflight_cache_creation_tokens, \
         preflight_cache_read_tokens, preflight_output_tokens, \
         probe_input_tokens, probe_cache_creation_tokens, \
         probe_cache_read_tokens, probe_output_tokens, \
         faithfulness_input_tokens, faithfulness_cache_creation_tokens, \
         faithfulness_cache_read_tokens, faithfulness_output_tokens, \
         original_replay_input_tokens, original_replay_cache_creation_tokens, \
         original_replay_cache_read_tokens, original_replay_output_tokens, \
         projected_replay_input_tokens, projected_replay_cache_creation_tokens, \
         projected_replay_cache_read_tokens, projected_replay_output_tokens, \
         judge_input_tokens, judge_cache_creation_tokens, \
         judge_cache_read_tokens, judge_output_tokens, \
         outcome, failure_stage, failure_kind, error_type) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
         ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, \
         ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44, ?45, ?46, ?47, ?48, \
         ?49, ?50, ?51, ?52, ?53, ?54, ?55, ?56, ?57, ?58, ?59, ?60, ?61, ?62, ?63, ?64, \
         ?65, ?66, ?67, ?68, ?69, ?70, ?71, ?72, ?73, ?74, ?75, ?76, ?77, ?78, ?79)",
    )
    .bind(&m.created_at)
    .bind(&m.quality_campaign_id)
    .bind(&m.h1_campaign_id)
    .bind(&m.case_id)
    .bind(source_kind)
    .bind(fixture_id)
    .bind(fixture_family)
    .bind(tag(QualityTag::SideEffectingOutput))
    .bind(tag(QualityTag::LongLog))
    .bind(tag(QualityTag::ExactError))
    .bind(tag(QualityTag::RejectedAlternative))
    .bind(tag(QualityTag::ParallelToolCycle))
    .bind(tag(QualityTag::PromptLikeToolText))
    .bind(tag(QualityTag::MutationOrOpenWork))
    .bind(&m.conversation_hash)
    .bind(&m.checkpoint_id)
    .bind(&m.source_boundary_hash)
    .bind(&m.summary_hash)
    .bind(&m.probe_set_hash)
    .bind(&m.original_plan_hash)
    .bind(&m.projected_plan_hash)
    .bind(&m.judge_hash)
    .bind(&m.task_model)
    .bind(&m.summarizer_response_model)
    .bind(&m.evaluator_response_model)
    .bind(QUALITY_METHOD_VERSION)
    .bind(QUALITY_RUBRIC_VERSION)
    .bind(PROBE_PROMPT_VERSION)
    .bind(FAITHFULNESS_PROMPT_VERSION)
    .bind(REPLAY_PROMPT_VERSION)
    .bind(JUDGE_PROMPT_VERSION)
    .bind(m.structural_pass.map(i64::from))
    .bind(m.claims_total)
    .bind(m.claims_supported)
    .bind(m.claims_unsupported)
    .bind(m.critical_hallucinations)
    .bind(m.noncritical_hallucinations)
    .bind(m.critical_omissions)
    .bind(m.noncritical_omissions)
    .bind(m.probes_total)
    .bind(m.probes_retained)
    .bind(m.probe_recall)
    .bind(m.original.map(|v| i64::from(v.correct)))
    .bind(m.original.map(|v| i64::from(v.constraint_adherent)))
    .bind(m.original.map(|v| i64::from(v.next_action_match)))
    .bind(m.projected.map(|v| i64::from(v.correct)))
    .bind(m.projected.map(|v| i64::from(v.constraint_adherent)))
    .bind(m.projected.map(|v| i64::from(v.next_action_match)))
    .bind(m.original.map(|v| i64::from(v.pass)))
    .bind(m.projected.map(|v| i64::from(v.pass)))
    .bind(m.comparison.map(ComparisonLabel::as_str))
    .bind(pf_in)
    .bind(pf_cc)
    .bind(pf_cr)
    .bind(pf_out)
    .bind(pr_in)
    .bind(pr_cc)
    .bind(pr_cr)
    .bind(pr_out)
    .bind(fa_in)
    .bind(fa_cc)
    .bind(fa_cr)
    .bind(fa_out)
    .bind(or_in)
    .bind(or_cc)
    .bind(or_cr)
    .bind(or_out)
    .bind(pj_in)
    .bind(pj_cc)
    .bind(pj_cr)
    .bind(pj_out)
    .bind(ju_in)
    .bind(ju_cc)
    .bind(ju_cr)
    .bind(ju_out)
    .bind(m.outcome.label())
    .bind(m.outcome.stage_label())
    .bind(m.outcome.failure().map(|f| f.kind.as_str()))
    .bind(
        m.outcome
            .failure()
            .and_then(|f| f.error_type)
            .map(UpstreamErrorType::as_str),
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Human-audit bucket (plan §"Human audit without raw persistence": four
/// automated accepts, four rejects, four nearest-threshold).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditBucket {
    Accepted,
    Rejected,
    NearThreshold,
}

impl AuditBucket {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::NearThreshold => "near_threshold",
        }
    }
}

/// Human verdict on one audited fixture case. A disagreement can never be
/// edited away — it forces a new rubric/prompt version and a new campaign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditVerdict {
    Agree,
    Disagree,
}

impl AuditVerdict {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agree => "agree",
            Self::Disagree => "disagree",
        }
    }
}

/// Record one human-audit verdict for a FIXTURE quality row. One-shot: the
/// `UNIQUE(quality_metric_id)` constraint rejects a second verdict for the
/// same case, and this module exposes no update or delete API. Live cases are
/// rejected before SQL — the audit reservoir is synthetic-fixture-only by
/// design.
pub async fn insert_audit(
    pool: &SqlitePool,
    quality_metric_id: i64,
    bucket: AuditBucket,
    verdict: AuditVerdict,
    created_at: &str,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    let source_kind: Option<String> =
        sqlx::query_scalar("SELECT source_kind FROM proxy_quality_metric WHERE id = ?1")
            .bind(quality_metric_id)
            .fetch_optional(&mut *tx)
            .await?;
    match source_kind.as_deref() {
        None => {
            return Err(protocol(format!(
                "audit references quality_metric_id {quality_metric_id}, which does not exist"
            )))
        }
        Some("fixture") => {}
        Some(_) => {
            return Err(protocol(
                "audits are synthetic-fixture-only; a live case can never be audited",
            ))
        }
    }
    sqlx::query(
        "INSERT INTO proxy_quality_audit \
         (created_at, quality_metric_id, audit_bucket, verdict, rubric_version) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(created_at)
    .bind(quality_metric_id)
    .bind(bucket.as_str())
    .bind(verdict.as_str())
    .bind(QUALITY_RUBRIC_VERSION)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Per-stage failure count (preflight + the five case stages).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct StageFailureCount {
    pub stage: String,
    pub count: i64,
}

/// Per-fixture-family completed-case count.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FamilyCount {
    pub family: String,
    pub count: i64,
}

/// Completed-case count per stratum tag.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub tag: &'static str,
    pub count: i64,
}

/// Usage totals for one role across the reporting scope.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleUsageTotal {
    pub role: &'static str,
    pub input_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
}

/// Campaign-scoped H2 quality report: denominators and failures, never
/// averages alone (plan §"`QualityReport` and binding GO predicate").
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityReport {
    pub terminal_cases: i64,
    pub completed: i64,
    pub preflight_ok: i64,
    pub preflight_failures: i64,
    pub structural_failures: i64,
    pub call_failures: i64,
    pub disarmed: i64,
    pub completed_live: i64,
    pub completed_fixture: i64,
    pub distinct_live_conversations: i64,
    pub fixture_family_counts: Vec<FamilyCount>,
    pub tag_counts: Vec<TagCount>,
    pub failures_by_stage: Vec<StageFailureCount>,
    pub usage_totals: Vec<RoleUsageTotal>,
    pub critical_hallucinations_total: i64,
    pub noncritical_hallucinations_total: i64,
    pub critical_omissions_total: i64,
    pub noncritical_omissions_total: i64,
    pub probes_total: i64,
    pub probes_retained: i64,
    /// `probes_retained / probes_total` over completed cases; `0.0` on empty.
    pub probe_recall_aggregate: f64,
    pub probe_recall_live: f64,
    pub probe_recall_fixture: f64,
    /// Minimum per-case recall, overall and per source (`1.0` when empty so
    /// an absent population never masquerades as a failing one; the count
    /// bars catch emptiness).
    pub probe_recall_min: f64,
    pub probe_recall_min_live: f64,
    pub probe_recall_min_fixture: f64,
    pub original_correct_rate: f64,
    pub original_constraint_rate: f64,
    pub original_next_action_rate: f64,
    pub projected_correct_rate: f64,
    pub projected_constraint_rate: f64,
    pub projected_next_action_rate: f64,
    pub original_pass_rate: f64,
    pub projected_pass_rate: f64,
    pub comparison_original_win: i64,
    pub comparison_tie: i64,
    pub comparison_projected_win: i64,
    pub comparison_both_fail: i64,
    /// Paired stratified cluster bootstrap over completed cases (live cluster
    /// = conversation, fixture cluster = family). Present only when the
    /// report is campaign-scoped (the seed derives from the campaign id) and
    /// the cluster minimums hold; `bootstrap_error` carries the bounded
    /// reason otherwise.
    pub behavior_point_estimate: Option<f64>,
    pub behavior_ci_lower: Option<f64>,
    pub behavior_ci_upper: Option<f64>,
    pub bootstrap_method_version: Option<String>,
    pub bootstrap_seed_hash: Option<String>,
    pub bootstrap_error: Option<String>,
    pub audits_total: i64,
    pub audit_accepted: i64,
    pub audit_rejected: i64,
    pub audit_near_threshold: i64,
    pub audit_agree: i64,
    pub audit_disagree: i64,
    /// Distinct stratum tags covered by audited cases (GO bar 10: all seven).
    pub audit_tags_covered: i64,
}

#[derive(sqlx::FromRow)]
struct QualityAgg {
    terminal_cases: i64,
    completed: i64,
    preflight_ok: i64,
    preflight_failures: i64,
    structural_failures: i64,
    call_failures: i64,
    disarmed: i64,
    completed_live: i64,
    completed_fixture: i64,
    distinct_live_conversations: i64,
    critical_hallucinations_total: i64,
    noncritical_hallucinations_total: i64,
    critical_omissions_total: i64,
    noncritical_omissions_total: i64,
    probes_total: i64,
    probes_retained: i64,
    probes_total_live: i64,
    probes_retained_live: i64,
    probes_total_fixture: i64,
    probes_retained_fixture: i64,
    probe_recall_min: f64,
    probe_recall_min_live: f64,
    probe_recall_min_fixture: f64,
    original_correct: i64,
    original_constraint: i64,
    original_next_action: i64,
    projected_correct: i64,
    projected_constraint: i64,
    projected_next_action: i64,
    original_pass_count: i64,
    projected_pass_count: i64,
    comparison_original_win: i64,
    comparison_tie: i64,
    comparison_projected_win: i64,
    comparison_both_fail: i64,
}

fn ratio(numerator: i64, denominator: i64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

/// Campaign-aware quality report. `campaign_id = None` aggregates every
/// campaign (the behavioral bootstrap is then omitted — its deterministic
/// seed derives from one campaign id); `since_hours = None` applies no time
/// cutoff, which is what the GO predicate requires.
pub async fn report(
    pool: &SqlitePool,
    since_hours: Option<i64>,
    campaign_id: Option<&str>,
) -> sqlx::Result<QualityReport> {
    let cutoff = match since_hours {
        Some(hours) => chrono::Utc::now()
            .checked_sub_signed(
                chrono::Duration::try_hours(hours.max(0)).unwrap_or(chrono::Duration::MAX),
            )
            .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
            .to_rfc3339(),
        None => chrono::DateTime::<chrono::Utc>::MIN_UTC.to_rfc3339(),
    };

    let agg = sqlx::query_as::<_, QualityAgg>(
        "SELECT \
           COUNT(*) AS terminal_cases, \
           COALESCE(SUM(outcome = 'completed'), 0) AS completed, \
           COALESCE(SUM(outcome = 'preflight_ok'), 0) AS preflight_ok, \
           COALESCE(SUM(outcome = 'preflight_failure'), 0) AS preflight_failures, \
           COALESCE(SUM(outcome = 'structural_failure'), 0) AS structural_failures, \
           COALESCE(SUM(outcome = 'call_failure'), 0) AS call_failures, \
           COALESCE(SUM(outcome = 'disarmed'), 0) AS disarmed, \
           COALESCE(SUM(outcome = 'completed' AND source_kind = 'live'), 0) AS completed_live, \
           COALESCE(SUM(outcome = 'completed' AND source_kind = 'fixture'), 0) \
             AS completed_fixture, \
           COUNT(DISTINCT CASE WHEN outcome = 'completed' AND source_kind = 'live' \
             THEN conversation_hash END) AS distinct_live_conversations, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN critical_hallucinations END), 0) \
             AS critical_hallucinations_total, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN noncritical_hallucinations END), 0) \
             AS noncritical_hallucinations_total, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN critical_omissions END), 0) \
             AS critical_omissions_total, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN noncritical_omissions END), 0) \
             AS noncritical_omissions_total, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN probes_total END), 0) \
             AS probes_total, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN probes_retained END), 0) \
             AS probes_retained, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' AND source_kind = 'live' \
             THEN probes_total END), 0) AS probes_total_live, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' AND source_kind = 'live' \
             THEN probes_retained END), 0) AS probes_retained_live, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' AND source_kind = 'fixture' \
             THEN probes_total END), 0) AS probes_total_fixture, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' AND source_kind = 'fixture' \
             THEN probes_retained END), 0) AS probes_retained_fixture, \
           COALESCE(MIN(CASE WHEN outcome = 'completed' THEN probe_recall END), 1.0) \
             AS probe_recall_min, \
           COALESCE(MIN(CASE WHEN outcome = 'completed' AND source_kind = 'live' \
             THEN probe_recall END), 1.0) AS probe_recall_min_live, \
           COALESCE(MIN(CASE WHEN outcome = 'completed' AND source_kind = 'fixture' \
             THEN probe_recall END), 1.0) AS probe_recall_min_fixture, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN original_correct END), 0) \
             AS original_correct, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN original_constraint_adherent END), 0) \
             AS original_constraint, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN original_next_action_match END), 0) \
             AS original_next_action, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN projected_correct END), 0) \
             AS projected_correct, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN projected_constraint_adherent END), 0) \
             AS projected_constraint, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN projected_next_action_match END), 0) \
             AS projected_next_action, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN original_pass END), 0) \
             AS original_pass_count, \
           COALESCE(SUM(CASE WHEN outcome = 'completed' THEN projected_pass END), 0) \
             AS projected_pass_count, \
           COALESCE(SUM(outcome = 'completed' AND comparison = 'original_win'), 0) \
             AS comparison_original_win, \
           COALESCE(SUM(outcome = 'completed' AND comparison = 'tie'), 0) AS comparison_tie, \
           COALESCE(SUM(outcome = 'completed' AND comparison = 'projected_win'), 0) \
             AS comparison_projected_win, \
           COALESCE(SUM(outcome = 'completed' AND comparison = 'both_fail'), 0) \
             AS comparison_both_fail \
         FROM proxy_quality_metric \
         WHERE created_at >= ?1 AND (?2 IS NULL OR quality_campaign_id = ?2)",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_one(pool)
    .await?;

    let fixture_family_counts: Vec<FamilyCount> = sqlx::query_as(
        "SELECT fixture_family AS family, COUNT(*) AS count FROM proxy_quality_metric \
         WHERE outcome = 'completed' AND source_kind = 'fixture' \
         AND created_at >= ?1 AND (?2 IS NULL OR quality_campaign_id = ?2) \
         GROUP BY fixture_family ORDER BY fixture_family",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_all(pool)
    .await?;

    type TagSums = (i64, i64, i64, i64, i64, i64, i64);
    let tag_sums: TagSums = sqlx::query_as(
        "SELECT \
           COALESCE(SUM(tag_side_effecting_output), 0), \
           COALESCE(SUM(tag_long_log), 0), \
           COALESCE(SUM(tag_exact_error), 0), \
           COALESCE(SUM(tag_rejected_alternative), 0), \
           COALESCE(SUM(tag_parallel_tool_cycle), 0), \
           COALESCE(SUM(tag_prompt_like_tool_text), 0), \
           COALESCE(SUM(tag_mutation_or_open_work), 0) \
         FROM proxy_quality_metric \
         WHERE outcome = 'completed' \
         AND created_at >= ?1 AND (?2 IS NULL OR quality_campaign_id = ?2)",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_one(pool)
    .await?;
    let tag_counts: Vec<TagCount> = QualityTag::ALL
        .into_iter()
        .zip([
            tag_sums.0, tag_sums.1, tag_sums.2, tag_sums.3, tag_sums.4, tag_sums.5, tag_sums.6,
        ])
        .map(|(tag, count)| TagCount {
            tag: tag.as_str(),
            count,
        })
        .collect();

    let failures_by_stage: Vec<StageFailureCount> = sqlx::query_as(
        "SELECT failure_stage AS stage, COUNT(*) AS count FROM proxy_quality_metric \
         WHERE failure_stage IS NOT NULL \
         AND created_at >= ?1 AND (?2 IS NULL OR quality_campaign_id = ?2) \
         GROUP BY failure_stage ORDER BY failure_stage",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_all(pool)
    .await?;

    #[derive(sqlx::FromRow)]
    struct UsageSums {
        pf_in: i64,
        pf_cc: i64,
        pf_cr: i64,
        pf_out: i64,
        pr_in: i64,
        pr_cc: i64,
        pr_cr: i64,
        pr_out: i64,
        fa_in: i64,
        fa_cc: i64,
        fa_cr: i64,
        fa_out: i64,
        or_in: i64,
        or_cc: i64,
        or_cr: i64,
        or_out: i64,
        pj_in: i64,
        pj_cc: i64,
        pj_cr: i64,
        pj_out: i64,
        ju_in: i64,
        ju_cc: i64,
        ju_cr: i64,
        ju_out: i64,
    }
    let u = sqlx::query_as::<_, UsageSums>(
        "SELECT \
           COALESCE(SUM(preflight_input_tokens), 0) AS pf_in, \
           COALESCE(SUM(preflight_cache_creation_tokens), 0) AS pf_cc, \
           COALESCE(SUM(preflight_cache_read_tokens), 0) AS pf_cr, \
           COALESCE(SUM(preflight_output_tokens), 0) AS pf_out, \
           COALESCE(SUM(probe_input_tokens), 0) AS pr_in, \
           COALESCE(SUM(probe_cache_creation_tokens), 0) AS pr_cc, \
           COALESCE(SUM(probe_cache_read_tokens), 0) AS pr_cr, \
           COALESCE(SUM(probe_output_tokens), 0) AS pr_out, \
           COALESCE(SUM(faithfulness_input_tokens), 0) AS fa_in, \
           COALESCE(SUM(faithfulness_cache_creation_tokens), 0) AS fa_cc, \
           COALESCE(SUM(faithfulness_cache_read_tokens), 0) AS fa_cr, \
           COALESCE(SUM(faithfulness_output_tokens), 0) AS fa_out, \
           COALESCE(SUM(original_replay_input_tokens), 0) AS or_in, \
           COALESCE(SUM(original_replay_cache_creation_tokens), 0) AS or_cc, \
           COALESCE(SUM(original_replay_cache_read_tokens), 0) AS or_cr, \
           COALESCE(SUM(original_replay_output_tokens), 0) AS or_out, \
           COALESCE(SUM(projected_replay_input_tokens), 0) AS pj_in, \
           COALESCE(SUM(projected_replay_cache_creation_tokens), 0) AS pj_cc, \
           COALESCE(SUM(projected_replay_cache_read_tokens), 0) AS pj_cr, \
           COALESCE(SUM(projected_replay_output_tokens), 0) AS pj_out, \
           COALESCE(SUM(judge_input_tokens), 0) AS ju_in, \
           COALESCE(SUM(judge_cache_creation_tokens), 0) AS ju_cc, \
           COALESCE(SUM(judge_cache_read_tokens), 0) AS ju_cr, \
           COALESCE(SUM(judge_output_tokens), 0) AS ju_out \
         FROM proxy_quality_metric \
         WHERE created_at >= ?1 AND (?2 IS NULL OR quality_campaign_id = ?2)",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_one(pool)
    .await?;
    let role_total = |role, i, cc, cr, o| RoleUsageTotal {
        role,
        input_tokens: i,
        cache_creation_tokens: cc,
        cache_read_tokens: cr,
        output_tokens: o,
    };
    let usage_totals = vec![
        role_total("preflight", u.pf_in, u.pf_cc, u.pf_cr, u.pf_out),
        role_total("probe", u.pr_in, u.pr_cc, u.pr_cr, u.pr_out),
        role_total("faithfulness", u.fa_in, u.fa_cc, u.fa_cr, u.fa_out),
        role_total("original_replay", u.or_in, u.or_cc, u.or_cr, u.or_out),
        role_total("projected_replay", u.pj_in, u.pj_cc, u.pj_cr, u.pj_out),
        role_total("judge", u.ju_in, u.ju_cc, u.ju_cr, u.ju_out),
    ];

    // Behavioral bootstrap over completed cases: live cluster = conversation,
    // fixture cluster = family. Campaign-scoped only — the deterministic seed
    // derives from the campaign id (Lane A pins the method).
    let (
        behavior_point_estimate,
        behavior_ci_lower,
        behavior_ci_upper,
        bootstrap_method_version,
        bootstrap_seed_hash,
        bootstrap_error,
    ) = if let Some(campaign) = campaign_id {
        type BehaviorRow = (String, Option<String>, String, i64, i64);
        let rows: Vec<BehaviorRow> = sqlx::query_as(
            "SELECT source_kind, fixture_family, conversation_hash, original_pass, \
             projected_pass FROM proxy_quality_metric \
             WHERE outcome = 'completed' AND created_at >= ?1 AND quality_campaign_id = ?2",
        )
        .bind(&cutoff)
        .bind(campaign)
        .fetch_all(pool)
        .await?;
        let cases: Vec<BehaviorCase> = rows
            .into_iter()
            .map(|(kind, family, conversation, original, projected)| {
                if kind == "fixture" {
                    BehaviorCase::fixture(family.unwrap_or_default(), original == 1, projected == 1)
                } else {
                    BehaviorCase::live(conversation, original == 1, projected == 1)
                }
            })
            .collect();
        match paired_cluster_bootstrap(campaign, QUALITY_RUBRIC_VERSION, &cases) {
            Ok(result) => (
                Some(result.point_estimate),
                Some(result.ci_lower),
                Some(result.ci_upper),
                Some(result.method_version.to_string()),
                Some(result.seed_hash),
                None,
            ),
            Err(error) => {
                let label = match error {
                    BootstrapError::NoCases => "no_cases",
                    BootstrapError::InsufficientLiveClusters => "insufficient_live_clusters",
                    BootstrapError::InsufficientFixtureFamilies => "insufficient_fixture_families",
                };
                (None, None, None, None, None, Some(label.to_string()))
            }
        }
    } else {
        (
            None,
            None,
            None,
            None,
            None,
            Some("not_campaign_scoped".to_string()),
        )
    };

    let audit: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
           COUNT(*), \
           COALESCE(SUM(a.audit_bucket = 'accepted'), 0), \
           COALESCE(SUM(a.audit_bucket = 'rejected'), 0), \
           COALESCE(SUM(a.audit_bucket = 'near_threshold'), 0), \
           COALESCE(SUM(a.verdict = 'agree'), 0), \
           COALESCE(SUM(a.verdict = 'disagree'), 0) \
         FROM proxy_quality_audit a \
         JOIN proxy_quality_metric m ON m.id = a.quality_metric_id \
         WHERE m.created_at >= ?1 AND (?2 IS NULL OR m.quality_campaign_id = ?2)",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_one(pool)
    .await?;

    // Distinct stratum tags covered by audited cases (GO bar 10). OR-fold the
    // seven booleans over audited rows in Rust — one small bounded query.
    type AuditTagRow = (i64, i64, i64, i64, i64, i64, i64);
    let audited_tags: Vec<AuditTagRow> = sqlx::query_as(
        "SELECT m.tag_side_effecting_output, m.tag_long_log, m.tag_exact_error, \
           m.tag_rejected_alternative, m.tag_parallel_tool_cycle, \
           m.tag_prompt_like_tool_text, m.tag_mutation_or_open_work \
         FROM proxy_quality_audit a \
         JOIN proxy_quality_metric m ON m.id = a.quality_metric_id \
         WHERE m.created_at >= ?1 AND (?2 IS NULL OR m.quality_campaign_id = ?2)",
    )
    .bind(&cutoff)
    .bind(campaign_id)
    .fetch_all(pool)
    .await?;
    let audit_tags_covered = {
        let mut covered = [false; 7];
        for (a, b, c, d, e, f, g) in &audited_tags {
            for (slot, value) in covered.iter_mut().zip([a, b, c, d, e, f, g]) {
                *slot |= *value == 1;
            }
        }
        covered.iter().filter(|c| **c).count() as i64
    };

    Ok(QualityReport {
        terminal_cases: agg.terminal_cases,
        completed: agg.completed,
        preflight_ok: agg.preflight_ok,
        preflight_failures: agg.preflight_failures,
        structural_failures: agg.structural_failures,
        call_failures: agg.call_failures,
        disarmed: agg.disarmed,
        completed_live: agg.completed_live,
        completed_fixture: agg.completed_fixture,
        distinct_live_conversations: agg.distinct_live_conversations,
        fixture_family_counts,
        tag_counts,
        failures_by_stage,
        usage_totals,
        critical_hallucinations_total: agg.critical_hallucinations_total,
        noncritical_hallucinations_total: agg.noncritical_hallucinations_total,
        critical_omissions_total: agg.critical_omissions_total,
        noncritical_omissions_total: agg.noncritical_omissions_total,
        probes_total: agg.probes_total,
        probes_retained: agg.probes_retained,
        probe_recall_aggregate: ratio(agg.probes_retained, agg.probes_total),
        probe_recall_live: ratio(agg.probes_retained_live, agg.probes_total_live),
        probe_recall_fixture: ratio(agg.probes_retained_fixture, agg.probes_total_fixture),
        probe_recall_min: agg.probe_recall_min,
        probe_recall_min_live: agg.probe_recall_min_live,
        probe_recall_min_fixture: agg.probe_recall_min_fixture,
        original_correct_rate: ratio(agg.original_correct, agg.completed),
        original_constraint_rate: ratio(agg.original_constraint, agg.completed),
        original_next_action_rate: ratio(agg.original_next_action, agg.completed),
        projected_correct_rate: ratio(agg.projected_correct, agg.completed),
        projected_constraint_rate: ratio(agg.projected_constraint, agg.completed),
        projected_next_action_rate: ratio(agg.projected_next_action, agg.completed),
        original_pass_rate: ratio(agg.original_pass_count, agg.completed),
        projected_pass_rate: ratio(agg.projected_pass_count, agg.completed),
        comparison_original_win: agg.comparison_original_win,
        comparison_tie: agg.comparison_tie,
        comparison_projected_win: agg.comparison_projected_win,
        comparison_both_fail: agg.comparison_both_fail,
        behavior_point_estimate,
        behavior_ci_lower,
        behavior_ci_upper,
        bootstrap_method_version,
        bootstrap_seed_hash,
        bootstrap_error,
        audits_total: audit.0,
        audit_accepted: audit.1,
        audit_rejected: audit.2,
        audit_near_threshold: audit.3,
        audit_agree: audit.4,
        audit_disagree: audit.5,
        audit_tags_covered,
    })
}

/// Per-bar GO verdict (plan's 11 binding conditions). Every bar is reported
/// individually so a NO-GO names exactly what failed — no bar can be averaged
/// away, and failed cases never enter a GO denominator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityGoBars {
    /// 1. At least 100 successful (completed) cases.
    pub min_cases: bool,
    /// 2. At least 30 live and at least 70 fixture completed cases.
    pub live_fixture_split: bool,
    /// 3. At least 10 completed cases in each of the seven strata.
    pub strata_minimums: bool,
    /// 4. Zero structural failures.
    pub zero_structural_failures: bool,
    /// 5. Zero critical hallucinations.
    pub zero_critical_hallucinations: bool,
    /// 6. Zero critical omissions.
    pub zero_critical_omissions: bool,
    /// 7. Aggregate, live, and fixture probe recall each >= 0.98.
    pub probe_recall: bool,
    /// 8. Behavioral paired 95% CI lower bound >= -0.05.
    pub behavioral_ci: bool,
    /// 9. At least two live conversation clusters and two fixture families
    ///    (the bootstrap fails closed otherwise, so a present CI implies it).
    pub cluster_minimums: bool,
    /// 10. All 12 synthetic audits complete (4/4/4, all seven tags) with zero
    ///     unresolved disagreements.
    pub audits_complete: bool,
    /// 11. The linked H1 campaign remains armed and `H1Gate::Pass` at report
    ///     time (both supplied by the caller from runtime state).
    pub h1_linked_pass: bool,
    /// Every bar above.
    pub go: bool,
}

/// Evaluate the binding H2 GO predicate over a campaign-scoped report.
/// `h1_armed` and `h1_gate` come from the caller: armed-ness is runtime
/// state, and the gate is `proxy_summary_metric::h1_gate` over the linked H1
/// campaign's own report. Passing authorizes only a human decision about H3
/// design — nothing else.
pub fn evaluate_go(
    report: &QualityReport,
    h1_armed: bool,
    h1_gate: crate::engine::repo::proxy_summary_metric::H1Gate,
) -> QualityGoBars {
    let min_cases = report.completed >= 100;
    let live_fixture_split = report.completed_live >= 30 && report.completed_fixture >= 70;
    let strata_minimums = report.tag_counts.len() == QualityTag::ALL.len()
        && report.tag_counts.iter().all(|t| t.count >= 10);
    let zero_structural_failures = report.structural_failures == 0;
    let zero_critical_hallucinations = report.critical_hallucinations_total == 0;
    let zero_critical_omissions = report.critical_omissions_total == 0;
    let probe_recall = report.probe_recall_aggregate >= 0.98
        && report.probe_recall_live >= 0.98
        && report.probe_recall_fixture >= 0.98;
    let behavioral_ci = report.behavior_ci_lower.is_some_and(|lower| lower >= -0.05);
    let cluster_minimums = report.bootstrap_error.is_none();
    let audits_complete = report.audits_total == 12
        && report.audit_accepted == 4
        && report.audit_rejected == 4
        && report.audit_near_threshold == 4
        && report.audit_tags_covered == QualityTag::ALL.len() as i64
        && report.audit_disagree == 0;
    let h1_linked_pass =
        h1_armed && h1_gate == crate::engine::repo::proxy_summary_metric::H1Gate::Pass;
    let go = min_cases
        && live_fixture_split
        && strata_minimums
        && zero_structural_failures
        && zero_critical_hallucinations
        && zero_critical_omissions
        && probe_recall
        && behavioral_ci
        && cluster_minimums
        && audits_complete
        && h1_linked_pass;
    QualityGoBars {
        min_cases,
        live_fixture_split,
        strata_minimums,
        zero_structural_failures,
        zero_critical_hallucinations,
        zero_critical_omissions,
        probe_recall,
        behavioral_ci,
        cluster_minimums,
        audits_complete,
        h1_linked_pass,
        go,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;
    use crate::engine::repo::proxy_summary_metric::H1Gate;

    fn fnv(seed: &str) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in seed.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    /// Deterministic, syntactically valid lowercase UUID fixture.
    fn fake_uuid(seed: &str) -> String {
        let h = format!("{:016x}{:016x}", fnv(seed), fnv(seed).rotate_left(17));
        format!(
            "{}-{}-{}-{}-{}",
            &h[0..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..32]
        )
    }

    /// 64 lowercase hex chars (SHA-256 shape).
    fn fake_hash(seed: &str) -> String {
        format!("{:016x}", fnv(seed)).repeat(4)
    }

    /// 16 lowercase hex chars (ctxopt stable_checkpoint_id shape).
    fn fake_checkpoint_id(seed: &str) -> String {
        format!("{:016x}", fnv(seed))
    }

    fn usage(seed: i64) -> RoleUsage {
        RoleUsage {
            input_tokens: 1_000 + seed,
            cache_creation_tokens: 100 + seed,
            cache_read_tokens: 10 + seed,
            output_tokens: seed,
        }
    }

    fn base_case(campaign: &str, case_seed: &str, outcome: QualityOutcome) -> QualityMetricInsert {
        QualityMetricInsert {
            created_at: chrono::Utc::now().to_rfc3339(),
            quality_campaign_id: fake_uuid(campaign),
            h1_campaign_id: fake_uuid("h1-campaign"),
            case_id: fake_uuid(case_seed),
            source: QualityCaseSource::Live,
            tags: vec![QualityTag::ExactError],
            conversation_hash: fake_hash("conv-1"),
            checkpoint_id: Some(fake_checkpoint_id("ckpt-1")),
            source_boundary_hash: Some(fake_hash("boundary-1")),
            summary_hash: Some(fake_hash("summary-1")),
            probe_set_hash: None,
            original_plan_hash: None,
            projected_plan_hash: None,
            judge_hash: None,
            task_model: "claude-task-x".into(),
            summarizer_response_model: Some("claude-task-x".into()),
            evaluator_response_model: None,
            structural_pass: None,
            claims_total: None,
            claims_supported: None,
            claims_unsupported: None,
            critical_hallucinations: None,
            noncritical_hallucinations: None,
            critical_omissions: None,
            noncritical_omissions: None,
            probes_total: None,
            probes_retained: None,
            probe_recall: None,
            original: None,
            projected: None,
            comparison: None,
            preflight_usage: None,
            probe_usage: None,
            faithfulness_usage: None,
            original_replay_usage: None,
            projected_replay_usage: None,
            judge_usage: None,
            outcome,
        }
    }

    fn verdict(pass: bool) -> PlanVerdict {
        PlanVerdict {
            correct: pass,
            constraint_adherent: pass,
            next_action_match: pass,
            pass,
        }
    }

    fn completed_case(
        campaign: &str,
        case_seed: &str,
        conversation_seed: &str,
        original_pass: bool,
        projected_pass: bool,
    ) -> QualityMetricInsert {
        let mut m = base_case(campaign, case_seed, QualityOutcome::Completed);
        m.conversation_hash = fake_hash(conversation_seed);
        m.tags = QualityTag::ALL.to_vec();
        m.evaluator_response_model = Some("claude-eval-y".into());
        m.probe_set_hash = Some(fake_hash("probes"));
        m.original_plan_hash = Some(fake_hash("orig-plan"));
        m.projected_plan_hash = Some(fake_hash("proj-plan"));
        m.judge_hash = Some(fake_hash("judge"));
        m.structural_pass = Some(true);
        m.claims_total = Some(20);
        m.claims_supported = Some(20);
        m.claims_unsupported = Some(0);
        m.critical_hallucinations = Some(0);
        m.noncritical_hallucinations = Some(0);
        m.critical_omissions = Some(0);
        m.noncritical_omissions = Some(1);
        m.probes_total = Some(10);
        m.probes_retained = Some(10);
        m.probe_recall = Some(1.0);
        m.original = Some(verdict(original_pass));
        m.projected = Some(verdict(projected_pass));
        m.comparison = Some(ComparisonLabel::expected(original_pass, projected_pass));
        m.probe_usage = Some(usage(1));
        m.faithfulness_usage = Some(usage(2));
        m.original_replay_usage = Some(usage(3));
        m.projected_replay_usage = Some(usage(4));
        m.judge_usage = Some(usage(5));
        m
    }

    fn fixture_case(
        campaign: &str,
        case_seed: &str,
        family: &str,
        original_pass: bool,
        projected_pass: bool,
    ) -> QualityMetricInsert {
        let mut m = completed_case(
            campaign,
            case_seed,
            "fixture-conv",
            original_pass,
            projected_pass,
        );
        m.source = QualityCaseSource::Fixture {
            id: format!("{family}-{case_seed}"),
            family: family.into(),
        };
        m
    }

    #[tokio::test]
    async fn completed_row_round_trips_with_versions_from_compiled_constants() {
        let pool = connect_in_memory().await;
        insert_terminal(
            &pool,
            completed_case("camp", "case-1", "conv-1", true, true),
        )
        .await
        .unwrap();
        type Row = (
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            Option<String>,
        );
        let (outcome, stage, kind, method, rubric, comparison): Row = sqlx::query_as(
            "SELECT outcome, failure_stage, failure_kind, method_version, rubric_version, \
             comparison FROM proxy_quality_metric",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(outcome, "completed");
        assert_eq!(stage, None);
        assert_eq!(kind, None);
        assert_eq!(method, QUALITY_METHOD_VERSION);
        assert_eq!(rubric, QUALITY_RUBRIC_VERSION);
        assert_eq!(comparison.as_deref(), Some("tie"));
    }

    #[tokio::test]
    async fn every_case_stage_and_kind_round_trips_through_the_check_constraints() {
        let pool = connect_in_memory().await;
        let mut n = 0;
        for stage in QualityCaseStage::ALL {
            for kind in QualityFailureKind::ALL {
                let error_type =
                    (kind == QualityFailureKind::Non2xx).then_some(UpstreamErrorType::ApiError);
                n += 1;
                insert_terminal(
                    &pool,
                    base_case(
                        "camp",
                        &format!("case-{n}"),
                        QualityOutcome::CallFailure(stage, QualityCallFailure { kind, error_type }),
                    ),
                )
                .await
                .unwrap_or_else(|e| panic!("stage {stage:?} kind {kind:?} rejected: {e}"));
            }
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM proxy_quality_metric WHERE outcome = 'call_failure'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 90, "5 stages x 18 kinds must all persist");
    }

    #[tokio::test]
    async fn every_upstream_error_type_round_trips_on_non_2xx() {
        let pool = connect_in_memory().await;
        for (i, error_type) in UpstreamErrorType::ALL.into_iter().enumerate() {
            insert_terminal(
                &pool,
                base_case(
                    "camp",
                    &format!("case-et-{i}"),
                    QualityOutcome::CallFailure(
                        QualityCaseStage::Judge,
                        QualityCallFailure {
                            kind: QualityFailureKind::Non2xx,
                            error_type: Some(error_type),
                        },
                    ),
                ),
            )
            .await
            .unwrap_or_else(|e| panic!("error_type {error_type:?} rejected: {e}"));
        }
        let distinct: i64 =
            sqlx::query_scalar("SELECT count(DISTINCT error_type) FROM proxy_quality_metric")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(distinct, 8);
    }

    #[tokio::test]
    async fn preflight_outcomes_round_trip() {
        let pool = connect_in_memory().await;
        let mut ok = base_case("camp", "case-pf-ok", QualityOutcome::PreflightOk);
        ok.preflight_usage = Some(usage(9));
        ok.evaluator_response_model = Some("claude-eval-y".into());
        insert_terminal(&pool, ok).await.unwrap();
        insert_terminal(
            &pool,
            base_case(
                "camp",
                "case-pf-fail",
                QualityOutcome::PreflightFailure(QualityCallFailure {
                    kind: QualityFailureKind::MissingAuth,
                    error_type: None,
                }),
            ),
        )
        .await
        .unwrap();
        let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT outcome, failure_stage, failure_kind FROM proxy_quality_metric ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows[0], ("preflight_ok".into(), None, None));
        assert_eq!(
            rows[1],
            (
                "preflight_failure".into(),
                Some("preflight".into()),
                Some("missing_auth".into())
            )
        );
    }

    #[tokio::test]
    async fn one_terminal_row_per_case_a_second_row_for_the_same_case_is_rejected() {
        let pool = connect_in_memory().await;
        insert_terminal(&pool, base_case("camp", "case-1", QualityOutcome::Disarmed))
            .await
            .unwrap();
        let second =
            insert_terminal(&pool, base_case("camp", "case-1", QualityOutcome::Disarmed)).await;
        assert!(
            second.is_err(),
            "UNIQUE(case_id) must reject a second terminal row"
        );
    }

    #[tokio::test]
    async fn error_type_with_a_non_non_2xx_kind_is_rejected() {
        let pool = connect_in_memory().await;
        let err = insert_terminal(
            &pool,
            base_case(
                "camp",
                "case-x",
                QualityOutcome::CallFailure(
                    QualityCaseStage::Probe,
                    QualityCallFailure {
                        kind: QualityFailureKind::Timeout,
                        error_type: Some(UpstreamErrorType::ApiError),
                    },
                ),
            ),
        )
        .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn completed_row_missing_measurements_or_inconsistent_labels_is_rejected() {
        let pool = connect_in_memory().await;

        let mut missing_faith = completed_case("camp", "case-1", "conv-1", true, true);
        missing_faith.claims_supported = None;
        assert!(insert_terminal(&pool, missing_faith).await.is_err());

        let mut bad_sum = completed_case("camp", "case-2", "conv-1", true, true);
        bad_sum.claims_supported = Some(19);
        assert!(insert_terminal(&pool, bad_sum).await.is_err());

        let mut bad_recall = completed_case("camp", "case-3", "conv-1", true, true);
        bad_recall.probe_recall = Some(0.5);
        assert!(insert_terminal(&pool, bad_recall).await.is_err());

        let mut both_fail_as_tie = completed_case("camp", "case-4", "conv-1", false, false);
        both_fail_as_tie.comparison = Some(ComparisonLabel::Tie);
        assert!(
            insert_terminal(&pool, both_fail_as_tie).await.is_err(),
            "a tie of two failures must not be recordable as a tie"
        );

        let mut structural = base_case("camp", "case-5", QualityOutcome::StructuralFailure);
        structural.structural_pass = None;
        assert!(insert_terminal(&pool, structural).await.is_err());

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM proxy_quality_metric")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "every rejected insert must persist nothing");
    }

    // Mellow's review bypass (ruling on hybrid-h2-lane-c): a forged
    // PlanVerdict whose overall pass contradicts its three dimension booleans
    // previously persisted through the real insert_terminal — and the GO
    // pass-rate columns read exactly those values. Both directions, both
    // sides, must now reject.
    #[tokio::test]
    async fn overall_pass_inconsistent_with_its_dimensions_is_rejected() {
        let pool = connect_in_memory().await;

        // The exact reproduced bypass: pass=true with all three dims false.
        let mut forged_pass = completed_case("camp", "case-1", "conv-1", true, true);
        forged_pass.original = Some(PlanVerdict {
            correct: false,
            constraint_adherent: false,
            next_action_match: false,
            pass: true,
        });
        assert!(
            insert_terminal(&pool, forged_pass).await.is_err(),
            "original pass=true with all dimensions false must be rejected"
        );

        // Inverse direction: a forged fail can't deflate the rate either.
        let mut forged_fail = completed_case("camp", "case-2", "conv-1", true, false);
        forged_fail.projected = Some(PlanVerdict {
            correct: true,
            constraint_adherent: true,
            next_action_match: true,
            pass: false,
        });
        assert!(
            insert_terminal(&pool, forged_fail).await.is_err(),
            "projected pass=false with all dimensions true must be rejected"
        );

        // One failing dimension with pass=true is equally forged.
        let mut one_dim = completed_case("camp", "case-3", "conv-1", true, true);
        one_dim.original = Some(PlanVerdict {
            correct: true,
            constraint_adherent: false,
            next_action_match: true,
            pass: true,
        });
        assert!(insert_terminal(&pool, one_dim).await.is_err());

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM proxy_quality_metric")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "no forged verdict may persist");
    }

    // PRIVACY (Global constraint #8): hostile raw content in any hash/id
    // field is rejected fail-closed, and nothing hostile reaches a committed
    // row's text columns.
    #[tokio::test]
    async fn hostile_raw_content_in_hash_and_id_fields_is_rejected_and_never_persisted() {
        let pool = connect_in_memory().await;
        const HOSTILE: &str =
            "RAW PROBE TEXT <<<FORGED-DELIMITER>>> sk-live-hostile secret prompt dump";

        let mut bad_campaign = completed_case("camp", "case-1", "conv-1", true, true);
        bad_campaign.quality_campaign_id = HOSTILE.into();
        assert!(insert_terminal(&pool, bad_campaign).await.is_err());

        let mut bad_case = completed_case("camp", "case-2", "conv-1", true, true);
        bad_case.case_id = HOSTILE.into();
        assert!(insert_terminal(&pool, bad_case).await.is_err());

        let mut bad_conv = completed_case("camp", "case-3", "conv-1", true, true);
        bad_conv.conversation_hash = HOSTILE.into();
        assert!(insert_terminal(&pool, bad_conv).await.is_err());

        let mut bad_probe_hash = completed_case("camp", "case-4", "conv-1", true, true);
        bad_probe_hash.probe_set_hash = Some(HOSTILE.into());
        assert!(insert_terminal(&pool, bad_probe_hash).await.is_err());

        let mut bad_judge_hash = completed_case("camp", "case-5", "conv-1", true, true);
        bad_judge_hash.judge_hash = Some(HOSTILE.into());
        assert!(insert_terminal(&pool, bad_judge_hash).await.is_err());

        let mut bad_ckpt = completed_case("camp", "case-6", "conv-1", true, true);
        bad_ckpt.checkpoint_id = Some(HOSTILE.into());
        assert!(insert_terminal(&pool, bad_ckpt).await.is_err());

        insert_terminal(
            &pool,
            completed_case("camp", "case-7", "conv-1", true, true),
        )
        .await
        .unwrap();
        // (quality_campaign_id, case_id, conversation_hash, probe_set_hash, judge_hash)
        type ScanRow = (String, String, String, Option<String>, Option<String>);
        let rows: Vec<ScanRow> = sqlx::query_as(
            "SELECT quality_campaign_id, case_id, conversation_hash, probe_set_hash, judge_hash \
             FROM proxy_quality_metric",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        let dump = format!("{rows:?}");
        assert!(
            !dump.contains("FORGED-DELIMITER"),
            "hostile marker leaked: {dump}"
        );
        assert!(
            !dump.contains("sk-live-hostile"),
            "credential marker leaked: {dump}"
        );
    }

    // PRIVACY backstop: raw SQL that bypasses the typed API entirely still
    // cannot commit forged labels or raw-shaped hash values — the DB CHECKs
    // are the last line.
    #[tokio::test]
    async fn db_checks_reject_forged_labels_and_raw_shaped_values_under_raw_sql() {
        let pool = connect_in_memory().await;
        let insert = |outcome: &'static str,
                      kind: Option<&'static str>,
                      error_type: Option<&'static str>,
                      conversation: &'static str,
                      source_kind: &'static str,
                      fixture_id: Option<&'static str>| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO proxy_quality_metric \
                     (created_at, quality_campaign_id, h1_campaign_id, case_id, source_kind, \
                      fixture_id, fixture_family, \
                      tag_side_effecting_output, tag_long_log, tag_exact_error, \
                      tag_rejected_alternative, tag_parallel_tool_cycle, \
                      tag_prompt_like_tool_text, tag_mutation_or_open_work, \
                      conversation_hash, task_model, method_version, rubric_version, \
                      probe_prompt_version, faithfulness_prompt_version, replay_prompt_version, \
                      judge_prompt_version, outcome, failure_stage, failure_kind, error_type) \
                     VALUES ('2026-01-01T00:00:00Z', \
                      '00000000-0000-0000-0000-000000000001', \
                      '00000000-0000-0000-0000-000000000002', \
                      lower(hex(randomblob(16))), \
                      ?1, ?2, ?2, 0, 0, 0, 0, 0, 0, 0, ?3, 'm', 'mv', 'rv', 'pv', 'fv', 'rpv', \
                      'jv', ?4, NULL, ?5, ?6)",
                )
                .bind(source_kind)
                .bind(fixture_id)
                .bind(conversation)
                .bind(outcome)
                .bind(kind)
                .bind(error_type)
                .execute(&pool)
                .await
            }
        };
        let valid_hash = "a".repeat(64);
        let valid_hash: &'static str = Box::leak(valid_hash.into_boxed_str());

        assert!(
            insert(
                "<script>alert(1)</script>",
                None,
                None,
                valid_hash,
                "live",
                None
            )
            .await
            .is_err(),
            "unknown outcome must violate the CHECK"
        );
        assert!(
            insert(
                "call_failure",
                Some("DROP TABLE"),
                None,
                valid_hash,
                "live",
                None
            )
            .await
            .is_err(),
            "unknown failure_kind must violate the CHECK"
        );
        assert!(
            insert(
                "call_failure",
                Some("timeout"),
                Some("leaked body text"),
                valid_hash,
                "live",
                None
            )
            .await
            .is_err(),
            "unlisted error_type must violate the CHECK"
        );
        assert!(
            insert(
                "call_failure",
                Some("timeout"),
                Some("api_error"),
                valid_hash,
                "live",
                None
            )
            .await
            .is_err(),
            "error_type without non_2xx must violate the coupling CHECK"
        );
        assert!(
            insert(
                "disarmed",
                None,
                None,
                "raw conversation text!",
                "live",
                None
            )
            .await
            .is_err(),
            "a raw-shaped conversation_hash must violate the shape CHECK"
        );
        assert!(
            insert("disarmed", None, None, valid_hash, "live", Some("fx-1"))
                .await
                .is_err(),
            "a live row with fixture identity must violate the coupling CHECK"
        );

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM proxy_quality_metric")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn audit_is_fixture_only_and_one_shot_with_no_update_api() {
        let pool = connect_in_memory().await;
        insert_terminal(
            &pool,
            fixture_case("camp", "case-fx", "exact-error", true, true),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            completed_case("camp", "case-live", "conv-1", true, true),
        )
        .await
        .unwrap();
        let (fixture_id, live_id): (i64, i64) = {
            let rows: Vec<(i64, String)> =
                sqlx::query_as("SELECT id, source_kind FROM proxy_quality_metric ORDER BY id")
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            let fixture = rows.iter().find(|r| r.1 == "fixture").unwrap().0;
            let live = rows.iter().find(|r| r.1 == "live").unwrap().0;
            (fixture, live)
        };

        insert_audit(
            &pool,
            fixture_id,
            AuditBucket::Accepted,
            AuditVerdict::Agree,
            "2026-07-12T10:00:00Z",
        )
        .await
        .unwrap();

        let second = insert_audit(
            &pool,
            fixture_id,
            AuditBucket::Accepted,
            AuditVerdict::Disagree,
            "2026-07-12T10:01:00Z",
        )
        .await;
        assert!(
            second.is_err(),
            "a second verdict for the same case must be rejected"
        );

        let live = insert_audit(
            &pool,
            live_id,
            AuditBucket::Accepted,
            AuditVerdict::Agree,
            "2026-07-12T10:02:00Z",
        )
        .await;
        assert!(live.is_err(), "live cases can never be audited");

        let missing = insert_audit(
            &pool,
            999_999,
            AuditBucket::Accepted,
            AuditVerdict::Agree,
            "2026-07-12T10:03:00Z",
        )
        .await;
        assert!(missing.is_err(), "an audit must reference an existing row");

        let verdict: String = sqlx::query_scalar("SELECT verdict FROM proxy_quality_audit")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(verdict, "agree", "the original verdict survives unchanged");
    }

    #[tokio::test]
    async fn report_splits_sources_computes_recall_and_wires_the_deterministic_bootstrap() {
        let pool = connect_in_memory().await;
        let campaign = fake_uuid("camp");

        // Two live conversations (clusters) + two fixture families.
        insert_terminal(
            &pool,
            completed_case("camp", "case-l1", "conv-a", true, true),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            completed_case("camp", "case-l2", "conv-b", true, false),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            fixture_case("camp", "case-f1", "exact-error", true, true),
        )
        .await
        .unwrap();
        insert_terminal(
            &pool,
            fixture_case("camp", "case-f2", "long-log", false, true),
        )
        .await
        .unwrap();
        // Failures must not enter completed denominators.
        insert_terminal(
            &pool,
            base_case(
                "camp",
                "case-cf",
                QualityOutcome::CallFailure(
                    QualityCaseStage::Faithfulness,
                    QualityCallFailure {
                        kind: QualityFailureKind::SchemaJson,
                        error_type: None,
                    },
                ),
            ),
        )
        .await
        .unwrap();
        let mut structural = base_case("camp", "case-sf", QualityOutcome::StructuralFailure);
        structural.structural_pass = Some(false);
        insert_terminal(&pool, structural).await.unwrap();

        let r = report(&pool, None, Some(&campaign)).await.unwrap();
        assert_eq!(r.completed, 4);
        assert_eq!(r.completed_live, 2);
        assert_eq!(r.completed_fixture, 2);
        assert_eq!(r.distinct_live_conversations, 2);
        assert_eq!(r.call_failures, 1);
        assert_eq!(r.structural_failures, 1);
        assert_eq!(r.fixture_family_counts.len(), 2);
        assert!((r.probe_recall_aggregate - 1.0).abs() < 1e-9);
        assert!((r.probe_recall_live - 1.0).abs() < 1e-9);
        assert!((r.probe_recall_fixture - 1.0).abs() < 1e-9);
        assert_eq!(r.comparison_tie, 2);
        assert_eq!(r.comparison_original_win, 1);
        assert_eq!(r.comparison_projected_win, 1);
        assert_eq!(
            r.failures_by_stage,
            vec![StageFailureCount {
                stage: "faithfulness".into(),
                count: 1
            }]
        );

        // Bootstrap present (2 live clusters + 2 families) and identical to a
        // direct call over the same cases — the report adds no hidden math.
        let expected = paired_cluster_bootstrap(
            &campaign,
            QUALITY_RUBRIC_VERSION,
            &[
                BehaviorCase::live(fake_hash("conv-a"), true, true),
                BehaviorCase::live(fake_hash("conv-b"), true, false),
                BehaviorCase::fixture("exact-error", true, true),
                BehaviorCase::fixture("long-log", false, true),
            ],
        )
        .unwrap();
        assert_eq!(r.behavior_point_estimate, Some(expected.point_estimate));
        assert_eq!(r.behavior_ci_lower, Some(expected.ci_lower));
        assert_eq!(r.behavior_ci_upper, Some(expected.ci_upper));
        assert_eq!(
            r.bootstrap_seed_hash.as_deref(),
            Some(expected.seed_hash.as_str())
        );
        assert_eq!(r.bootstrap_error, None);

        // Cross-campaign report omits the bootstrap with a bounded label.
        let all = report(&pool, None, None).await.unwrap();
        assert_eq!(all.bootstrap_error.as_deref(), Some("not_campaign_scoped"));
        assert_eq!(all.behavior_ci_lower, None);
    }

    #[tokio::test]
    async fn report_on_empty_table_is_all_zero_with_bounded_bootstrap_error() {
        let pool = connect_in_memory().await;
        let r = report(&pool, Some(24), Some(&fake_uuid("camp")))
            .await
            .unwrap();
        assert_eq!(r.terminal_cases, 0);
        assert_eq!(r.completed, 0);
        assert_eq!(r.probe_recall_aggregate, 0.0);
        assert_eq!(r.bootstrap_error.as_deref(), Some("no_cases"));
        assert_eq!(r.audits_total, 0);
        assert_eq!(r.audit_tags_covered, 0);
    }

    /// A synthetic passing report for the GO-bar table tests. Every bar
    /// passes; each test flips exactly one input and asserts exactly that bar
    /// (and `go`) turns false — so no bar can be silently absorbed by another.
    fn passing_report() -> QualityReport {
        QualityReport {
            terminal_cases: 101,
            completed: 100,
            preflight_ok: 1,
            preflight_failures: 0,
            structural_failures: 0,
            call_failures: 0,
            disarmed: 0,
            completed_live: 30,
            completed_fixture: 70,
            distinct_live_conversations: 10,
            fixture_family_counts: vec![
                FamilyCount {
                    family: "a".into(),
                    count: 35,
                },
                FamilyCount {
                    family: "b".into(),
                    count: 35,
                },
            ],
            tag_counts: QualityTag::ALL
                .into_iter()
                .map(|tag| TagCount {
                    tag: tag.as_str(),
                    count: 15,
                })
                .collect(),
            failures_by_stage: vec![],
            usage_totals: vec![],
            critical_hallucinations_total: 0,
            noncritical_hallucinations_total: 3,
            critical_omissions_total: 0,
            noncritical_omissions_total: 2,
            probes_total: 1_000,
            probes_retained: 990,
            probe_recall_aggregate: 0.99,
            probe_recall_live: 0.99,
            probe_recall_fixture: 0.99,
            probe_recall_min: 0.9,
            probe_recall_min_live: 0.9,
            probe_recall_min_fixture: 0.9,
            original_correct_rate: 0.95,
            original_constraint_rate: 0.95,
            original_next_action_rate: 0.95,
            projected_correct_rate: 0.95,
            projected_constraint_rate: 0.95,
            projected_next_action_rate: 0.95,
            original_pass_rate: 0.9,
            projected_pass_rate: 0.9,
            comparison_original_win: 5,
            comparison_tie: 90,
            comparison_projected_win: 5,
            comparison_both_fail: 0,
            behavior_point_estimate: Some(0.0),
            behavior_ci_lower: Some(-0.03),
            behavior_ci_upper: Some(0.03),
            bootstrap_method_version: Some("paired-bootstrap-v1".into()),
            bootstrap_seed_hash: Some("00".repeat(32)),
            bootstrap_error: None,
            audits_total: 12,
            audit_accepted: 4,
            audit_rejected: 4,
            audit_near_threshold: 4,
            audit_agree: 12,
            audit_disagree: 0,
            audit_tags_covered: 7,
        }
    }

    #[test]
    fn go_passes_only_when_every_bar_passes() {
        let bars = evaluate_go(&passing_report(), true, H1Gate::Pass);
        assert!(
            bars.go,
            "the reference report must pass every bar: {bars:?}"
        );
    }

    #[test]
    fn each_go_bar_fails_independently_and_fails_the_overall_go() {
        // (mutator, name, extractor for the single bar expected to flip)
        type Case = (
            fn(&mut QualityReport),
            &'static str,
            fn(&QualityGoBars) -> bool,
        );
        let cases: [Case; 10] = [
            (|r| r.completed = 99, "min_cases", |b| b.min_cases),
            (
                |r| {
                    r.completed_live = 29;
                    r.completed_fixture = 71;
                },
                "live_fixture_split",
                |b| b.live_fixture_split,
            ),
            (
                |r| r.tag_counts[3].count = 9,
                "strata_minimums",
                |b| b.strata_minimums,
            ),
            (
                |r| r.structural_failures = 1,
                "zero_structural_failures",
                |b| b.zero_structural_failures,
            ),
            (
                |r| r.critical_hallucinations_total = 1,
                "zero_critical_hallucinations",
                |b| b.zero_critical_hallucinations,
            ),
            (
                |r| r.critical_omissions_total = 1,
                "zero_critical_omissions",
                |b| b.zero_critical_omissions,
            ),
            (
                |r| r.probe_recall_live = 0.979,
                "probe_recall (live below 0.98 even with agg passing)",
                |b| b.probe_recall,
            ),
            (
                |r| r.behavior_ci_lower = Some(-0.0501),
                "behavioral_ci",
                |b| b.behavioral_ci,
            ),
            (
                |r| {
                    r.bootstrap_error = Some("insufficient_live_clusters".into());
                    r.behavior_ci_lower = None;
                },
                "cluster_minimums",
                |b| b.cluster_minimums,
            ),
            (
                |r| r.audit_disagree = 1,
                "audits_complete",
                |b| b.audits_complete,
            ),
        ];
        for (mutate, name, bar) in cases {
            let mut r = passing_report();
            mutate(&mut r);
            let bars = evaluate_go(&r, true, H1Gate::Pass);
            assert!(!bar(&bars), "bar `{name}` must fail: {bars:?}");
            assert!(!bars.go, "overall go must fail when `{name}` fails");
        }
    }

    #[test]
    fn go_bar_11_requires_armed_and_h1_pass_at_report_time() {
        let r = passing_report();
        assert!(!evaluate_go(&r, false, H1Gate::Pass).h1_linked_pass);
        assert!(!evaluate_go(&r, true, H1Gate::Inconclusive).h1_linked_pass);
        assert!(!evaluate_go(&r, true, H1Gate::Fail).h1_linked_pass);
        assert!(!evaluate_go(&r, false, H1Gate::Fail).go);
    }

    #[test]
    fn go_recall_boundary_is_inclusive_at_exactly_098() {
        let mut r = passing_report();
        r.probe_recall_aggregate = 0.98;
        r.probe_recall_live = 0.98;
        r.probe_recall_fixture = 0.98;
        assert!(evaluate_go(&r, true, H1Gate::Pass).probe_recall);
    }

    #[test]
    fn go_behavioral_boundary_is_inclusive_at_exactly_minus_005() {
        let mut r = passing_report();
        r.behavior_ci_lower = Some(-0.05);
        assert!(evaluate_go(&r, true, H1Gate::Pass).behavioral_ci);
    }

    #[test]
    fn go_audit_bar_requires_all_tags_and_exact_buckets() {
        let mut wrong_tags = passing_report();
        wrong_tags.audit_tags_covered = 6;
        assert!(!evaluate_go(&wrong_tags, true, H1Gate::Pass).audits_complete);

        let mut wrong_buckets = passing_report();
        wrong_buckets.audit_accepted = 5;
        wrong_buckets.audit_rejected = 3;
        assert!(!evaluate_go(&wrong_buckets, true, H1Gate::Pass).audits_complete);
    }
}

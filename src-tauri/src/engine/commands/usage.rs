//! `usage.overview` — the frozen read side of measured model usage
//! (docs/plans/2026-09-05-usage-engine.md, contract
//! docs/plans/2026-09-05-usage-overview-contract.md).
//!
//! This module answers ONE question: for a filter set and a trailing 30/90
//! calendar dates in a real IANA zone, what activity was recorded, how many
//! tokens were actually measured, and — the part that keeps it honest — how
//! much of that window was observed at all.
//!
//! # Three rules the assembly exists to enforce
//!
//! 1. **Missing is not zero.** `measuredTokens` is a number only when events
//!    with known tokens were summed, or when the window was verifiably fully
//!    observed AND held no activity (plan D4). Everything else is `null`.
//! 2. **An event does not prove coverage.** Coverage comes from
//!    `model_usage_coverage` intervals, and `complete` demands gap-free
//!    observation of EVERY source in [`COLLECTED_SOURCES`] across the whole
//!    interval — never "whichever sources happen to have rows"
//!    (Aoki, Foundation assembly clarification).
//! 3. **Nothing is silently re-attributed.** A Library draft with no workspace
//!    surfaces under the wire-only [`UNSCOPED_WORKSPACE_ID`], never under the
//!    selected workspace; a requested model never impersonates a served one.
//!
//! No filesystem scanning, no raw transcript rows, no prompt text: the whole
//! handler is bounded SQL over indexed timestamps plus small metadata reads.

use crate::engine::repo::model_usage::{
    self, GroupColumn, ModelKeyFilter, UsageAggregate, UsageScope, UNSUPPORTED_SOURCE,
};
use crate::engine::{repo, AppError, AppState};
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

// ── Reserved wire identities (preflight correction b1140da9) ─────────────────

/// Wire-only workspace id for activity that legitimately has NO workspace (a
/// Library draft run). Never a database row, never accepted by a workspace
/// lifecycle command.
pub const UNSCOPED_WORKSPACE_ID: &str = "__unscoped__";
const UNSCOPED_WORKSPACE_NAME: &str = "No workspace";
/// Wire-only agent id for activity with no workspace-agent attribution.
pub const UNASSIGNED_AGENT_ID: &str = "__unassigned__";
const UNASSIGNED_AGENT_NAME: &str = "Unassigned activity";

/// Displayed name of a model identity the source never proved.
const UNKNOWN_MODEL_NAME: &str = "Unknown model";

// ── Sources this build collects ──────────────────────────────────────────────

/// Claude Code transcript importer.
pub const SOURCE_CLAUDE_TRANSCRIPT: &str = "claude-code";
/// Codex transcript importer.
pub const SOURCE_CODEX_TRANSCRIPT: &str = "codex";
/// Direct provider chat (`runtime::chat`).
pub const SOURCE_DIRECT_CHAT: &str = "chat";
/// Fusion panel/judge/synthesis provider calls.
pub const SOURCE_FUSION: &str = "fusion";
/// Non-persistent one-shot draft invocations.
pub const SOURCE_DRAFT: &str = "draft";

/// The source set a `complete` answer must account for.
///
/// This list is DECLARED, not derived from the coverage table: inferring the
/// required sources from whichever rows exist would let a single collector's
/// interval certify a window nobody else observed. A source with no interval
/// covering the window keeps the answer `partial`, which is the honest reading
/// of "we never heard from that collector".
pub const COLLECTED_SOURCES: &[&str] = &[
    SOURCE_CLAUDE_TRANSCRIPT,
    SOURCE_CODEX_TRANSCRIPT,
    SOURCE_DIRECT_CHAT,
    SOURCE_FUSION,
    SOURCE_DRAFT,
];

// ── Request ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OverviewReq {
    days: i64,
    time_zone: String,
    workspace_id: Option<String>,
    workspace_agent_id: Option<String>,
    model_key: Option<String>,
}

// ── Wire shapes ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageTotals {
    activity_count: i64,
    response_count: i64,
    invocation_count: i64,
    /// `null` unless measured events were summed or the window was verifiably
    /// complete AND empty (plan D4).
    measured_tokens: Option<i64>,
    measured_event_count: i64,
    unknown_usage_count: i64,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    coverage: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageDay {
    #[serde(flatten)]
    totals: UsageTotals,
    date: String,
    start_utc: String,
    end_utc: String,
    in_progress: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageModelOption {
    key: String,
    name: String,
    provider: Option<String>,
    basis: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageWorkspaceOption {
    id: String,
    name: String,
    archived: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageAgentOption {
    id: String,
    name: String,
    workspace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UsageModelRow {
    #[serde(flatten)]
    totals: UsageTotals,
    #[serde(flatten)]
    option: UsageModelOption,
}

#[derive(Debug, Clone, Serialize)]
struct UsageAgentRow {
    #[serde(flatten)]
    totals: UsageTotals,
    #[serde(flatten)]
    option: UsageAgentOption,
}

#[derive(Debug, Clone, Serialize)]
struct UsageWorkspaceRow {
    #[serde(flatten)]
    totals: UsageTotals,
    #[serde(flatten)]
    option: UsageWorkspaceOption,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageContext {
    workspace_agent_id: String,
    agent_name: String,
    workspace_id: String,
    workspace_name: String,
    archived: bool,
    model_key: String,
    tokens: Option<i64>,
    capacity: Option<i64>,
    source: Option<String>,
    observed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageRange {
    days: i64,
    time_zone: String,
    start_date: String,
    end_date: String,
    start_utc: String,
    end_utc: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageCoverageBlock {
    state: &'static str,
    collecting_since: Option<String>,
    last_verified_at: Option<String>,
    pending_import: bool,
    unsupported_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageOverview {
    generated_at: String,
    range: UsageRange,
    summary: UsageTotals,
    daily: Vec<UsageDay>,
    models: Vec<UsageModelOption>,
    agents: Vec<UsageAgentOption>,
    workspaces: Vec<UsageWorkspaceOption>,
    by_model: Vec<UsageModelRow>,
    by_agent: Vec<UsageAgentRow>,
    by_workspace: Vec<UsageWorkspaceRow>,
    contexts: Vec<UsageContext>,
    coverage: UsageCoverageBlock,
}

// ── Model identity ───────────────────────────────────────────────────────────

/// Basis of a model identity. `reported` is a model the source OBSERVED
/// serving the response; `selected` is only what was requested. They are
/// separate identities on purpose — a selected model must never be presented
/// as a served one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Basis {
    Reported,
    Selected,
    Unknown,
}

impl Basis {
    fn wire(self) -> &'static str {
        match self {
            Basis::Reported => "reported",
            Basis::Selected => "selected",
            Basis::Unknown => "unknown",
        }
    }

    fn parse(text: &str) -> Option<Basis> {
        match text {
            "reported" => Some(Basis::Reported),
            "selected" => Some(Basis::Selected),
            "unknown" => Some(Basis::Unknown),
            _ => None,
        }
    }
}

/// One model identity: provider + name + basis.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ModelIdentity {
    basis: Basis,
    provider: Option<String>,
    name: Option<String>,
}

impl ModelIdentity {
    /// From a stored event's two model columns: a served model wins and is
    /// `reported`; otherwise a requested model is `selected`; neither is
    /// `unknown` (and an unknown identity carries no provider, so all unknown
    /// events share one key).
    fn from_event(
        provider: Option<String>,
        served: Option<String>,
        requested: Option<String>,
    ) -> ModelIdentity {
        match (served, requested) {
            (Some(name), _) => ModelIdentity {
                basis: Basis::Reported,
                provider,
                name: Some(name),
            },
            (None, Some(name)) => ModelIdentity {
                basis: Basis::Selected,
                provider,
                name: Some(name),
            },
            (None, None) => ModelIdentity {
                basis: Basis::Unknown,
                provider: None,
                name: None,
            },
        }
    }

    /// Opaque, stable, round-trippable encoding: `basis:provider:name`.
    ///
    /// Each optional component is `""` for absent or `"="` + escaped text for
    /// present, so `None` and `Some("")` stay distinguishable; `\` and `:`
    /// inside a value are escaped, so a model name containing a colon cannot
    /// forge a different key.
    fn key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.basis.wire(),
            encode_opt(self.provider.as_deref()),
            encode_opt(self.name.as_deref())
        )
    }

    fn option(&self) -> UsageModelOption {
        UsageModelOption {
            key: self.key(),
            name: self
                .name
                .clone()
                .unwrap_or_else(|| UNKNOWN_MODEL_NAME.to_string()),
            provider: self.provider.clone(),
            basis: self.basis.wire(),
        }
    }
}

fn encode_opt(value: Option<&str>) -> String {
    match value {
        None => String::new(),
        Some(text) => {
            let mut out = String::from("=");
            for ch in text.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    ':' => out.push_str("\\:"),
                    other => out.push(other),
                }
            }
            out
        }
    }
}

/// Split an encoded key on UNESCAPED colons, then decode each component.
fn decode_key(key: &str) -> Option<ModelIdentity> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for ch in key.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ':' {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    if escaped {
        return None; // trailing lone backslash
    }
    parts.push(current);
    if parts.len() != 3 {
        return None;
    }
    let basis = Basis::parse(&parts[0])?;
    let decode_opt = |raw: &str| -> Option<Option<String>> {
        if raw.is_empty() {
            return Some(None);
        }
        raw.strip_prefix('=').map(|rest| Some(rest.to_string()))
    };
    let provider = decode_opt(&parts[1])?;
    let name = decode_opt(&parts[2])?;
    if basis == Basis::Unknown && (provider.is_some() || name.is_some()) {
        return None; // an unknown identity has no provider and no name
    }
    if basis != Basis::Unknown && name.is_none() {
        return None; // a reported/selected identity always names a model
    }
    Some(ModelIdentity {
        basis,
        provider,
        name,
    })
}

/// Provider id for an agent's CONFIGURED model (the context gauge's identity).
///
/// A chat agent names its provider directly. A CLI agent does not, so the
/// vendor is derived from its harness — the same mapping the transcript
/// collectors use, so a context and the events it produced share one key.
fn provider_for_agent(provider_id: Option<&str>, cli_kind: Option<&str>) -> Option<String> {
    if let Some(id) = provider_id.filter(|p| !p.is_empty()) {
        return Some(id.to_string());
    }
    match cli_kind {
        Some("claude-code") => Some("anthropic".to_string()),
        Some("codex") => Some("openai".to_string()),
        _ => None,
    }
}

// ── Scope selection ──────────────────────────────────────────────────────────

/// One dimension of a query's scope.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sel {
    /// No filter on this dimension.
    All,
    /// Exactly this id.
    Id(String),
    /// Only rows with NO value here (`__unscoped__` / `__unassigned__`).
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopeQuery {
    workspace: Sel,
    agent: Sel,
}

/// Does a coverage/cursor row's scope PROVE something about this query's
/// scope? Only if the row is at least as WIDE: a `NULL` dimension on a
/// coverage row means "unrestricted observation", so it proves anything, while
/// a row naming one workspace proves nothing about a query spanning all of
/// them (Aoki: "a compatible narrower row can establish partial observation
/// but cannot alone prove a broader query complete").
fn row_proves(row_scope: Option<&str>, sel: &Sel) -> bool {
    match row_scope {
        None => true,
        Some(id) => matches!(sel, Sel::Id(want) if want == id),
    }
}

/// Could this row's scope contain events the query selects? Compatible rows
/// are evidence that observation happened (→ `partial`), not proof of
/// completeness.
fn row_compatible(row_scope: Option<&str>, sel: &Sel) -> bool {
    match row_scope {
        None => true,
        Some(id) => match sel {
            Sel::All => true,
            Sel::Id(want) => want == id,
            // A row scoped to a workspace/agent observed attributed activity;
            // it says nothing about the unattributed bucket.
            Sel::Null => false,
        },
    }
}

// ── Coverage ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Coverage {
    None,
    Partial,
    Complete,
}

impl Coverage {
    fn wire(self) -> &'static str {
        match self {
            Coverage::None => "none",
            Coverage::Partial => "partial",
            Coverage::Complete => "complete",
        }
    }
}

/// A coverage interval with its bounds already parsed. A row whose timestamps
/// do not parse is dropped before this point: an unreadable interval proves
/// nothing, and dropping it can only make the answer more conservative.
#[derive(Debug, Clone)]
struct CoverageInterval {
    workspace_id: Option<String>,
    workspace_agent_id: Option<String>,
    source_kind: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    complete: bool,
    unsupported: bool,
    /// A partial interval that carries ANY diagnostic: the collector saw the
    /// window and knows the observation was damaged (unsupported shape, invalid
    /// counters, a conflict). Such a row caps a compatible scope at partial even
    /// when complete spans cover the same window (ruling on 34201f49).
    damaged: bool,
    last_verified_at: String,
}

impl CoverageInterval {
    fn overlaps(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        self.start < end && self.end > start
    }
}

/// Every compatible interval overlapping the window — the evidence pool for
/// one scope.
fn compatible<'a>(
    intervals: &'a [CoverageInterval],
    scope: &ScopeQuery,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<&'a CoverageInterval> {
    intervals
        .iter()
        .filter(|row| {
            row.overlaps(start, end)
                && row_compatible(row.workspace_id.as_deref(), &scope.workspace)
                && row_compatible(row.workspace_agent_id.as_deref(), &scope.agent)
        })
        .collect()
}

/// Do these intervals cover `[start, end)` without a gap?
fn covers_without_gap(
    mut spans: Vec<(DateTime<Utc>, DateTime<Utc>)>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> bool {
    if start >= end {
        return false;
    }
    spans.sort_by_key(|(s, _)| *s);
    let mut reached = start;
    for (s, e) in spans {
        if s > reached {
            return false; // a gap the union cannot close
        }
        if e > reached {
            reached = e;
        }
        if reached >= end {
            return true;
        }
    }
    reached >= end
}

/// Resolve the coverage state of one window for one scope.
///
/// `complete` requires all three of: at least one compatible observation, no
/// unsupported-source diagnostic in the window, and — for EVERY source in
/// [`COLLECTED_SOURCES`] — a gap-free union of *proving* complete intervals.
fn coverage_for(
    intervals: &[CoverageInterval],
    scope: &ScopeQuery,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Coverage {
    let evidence = compatible(intervals, scope, start, end);
    if evidence.is_empty() {
        return Coverage::None;
    }
    if evidence.iter().any(|row| row.unsupported || row.damaged) {
        return Coverage::Partial;
    }
    for source in COLLECTED_SOURCES {
        let spans: Vec<(DateTime<Utc>, DateTime<Utc>)> = evidence
            .iter()
            .filter(|row| {
                row.source_kind == *source
                    && row.complete
                    && row_proves(row.workspace_id.as_deref(), &scope.workspace)
                    && row_proves(row.workspace_agent_id.as_deref(), &scope.agent)
            })
            .map(|row| (row.start, row.end))
            .collect();
        if !covers_without_gap(spans, start, end) {
            return Coverage::Partial;
        }
    }
    Coverage::Complete
}

/// Source kinds a collector explicitly could not import inside this window and
/// scope. These also prevent `complete` (they are unobserved by definition).
fn unsupported_sources(
    intervals: &[CoverageInterval],
    scope: &ScopeQuery,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for row in compatible(intervals, scope, start, end) {
        if row.unsupported {
            seen.insert(row.source_kind.clone());
        }
    }
    seen.into_iter().collect()
}

// ── Calendar buckets ─────────────────────────────────────────────────────────

/// The UTC instant a local calendar date begins.
///
/// DST makes this fiddly in exactly two ways, and both are handled here rather
/// than in SQL (SQLite has no timezone database at all):
/// * **Ambiguous** (clocks went back over midnight): the FIRST of the two
///   instants — the day starts the moment the local date first appears.
/// * **Skipped** (clocks went forward over midnight, e.g. America/Havana in
///   spring): there is no local 00:00, so the day starts at the transition
///   instant, found by walking forward a minute at a time.
fn local_day_start(tz: Tz, date: NaiveDate) -> Option<DateTime<Utc>> {
    for minutes in 0..(6 * 60) {
        let naive = date.and_hms_opt(0, 0, 0)? + Duration::minutes(minutes);
        if naive.date() != date {
            return None;
        }
        match tz.from_local_datetime(&naive) {
            chrono::LocalResult::Single(dt) => return Some(dt.with_timezone(&Utc)),
            chrono::LocalResult::Ambiguous(first, _) => return Some(first.with_timezone(&Utc)),
            chrono::LocalResult::None => continue,
        }
    }
    None
}

/// One daily bucket: a half-open UTC interval carrying its local date.
#[derive(Debug, Clone)]
struct Bucket {
    date: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    in_progress: bool,
}

/// The trailing `days` calendar dates ending with today in `tz`, ascending.
///
/// Today's bucket ends at `now`, not at tomorrow's midnight: an interval that
/// has not happened yet is not part of the query.
fn build_buckets(tz: Tz, days: i64, now: DateTime<Utc>) -> Result<Vec<Bucket>, AppError> {
    let today = now.with_timezone(&tz).date_naive();
    let first = today - Duration::days(days - 1);
    let mut buckets = Vec::with_capacity(days as usize);
    let mut date = first;
    while date <= today {
        let start = local_day_start(tz, date)
            .ok_or_else(|| AppError::Internal(format!("no local start for {date}")))?;
        let next = date + Duration::days(1);
        let raw_end = local_day_start(tz, next)
            .ok_or_else(|| AppError::Internal(format!("no local start for {next}")))?;
        let in_progress = date == today;
        let end = if in_progress { now } else { raw_end };
        buckets.push(Bucket {
            date: format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day()),
            start,
            end,
            in_progress,
        });
        date = next;
    }
    Ok(buckets)
}

// ── Totals assembly ──────────────────────────────────────────────────────────

const EMPTY_AGGREGATE: UsageAggregate = UsageAggregate {
    bucket: String::new(),
    activity_count: 0,
    response_count: 0,
    invocation_count: 0,
    measured_event_count: 0,
    unknown_usage_count: 0,
    measured_tokens: None,
    input_tokens: None,
    output_tokens: None,
    measured_overflow: false,
    input_overflow: false,
    output_overflow: false,
    rejected_counter_count: 0,
    conflict_count: 0,
};

/// Plan D4, in one place: a measured number, a verified zero, or `null`.
///
/// `known` is the value the query actually summed. It is returned as-is when
/// measured events exist (a summed 0 IS a measurement) — unless the exact sum
/// did not fit, in which case it is unavailable rather than 0 or rounded. With
/// no measured event, only a verifiably complete AND empty window may report 0
/// — coverage proves events were observed, never that a missing component was
/// zero.
fn measured_value(
    known: Option<i64>,
    overflow: bool,
    agg: &UsageAggregate,
    coverage: Coverage,
) -> Option<i64> {
    if overflow {
        return None;
    }
    if agg.measured_event_count > 0 {
        return Some(known.unwrap_or(0));
    }
    if agg.activity_count == 0 && coverage == Coverage::Complete {
        return Some(0);
    }
    None
}

/// The coverage a group may claim once its own rows are taken into account.
///
/// Observation coverage says the collectors watched the window; a rejected
/// counter, a conflicting response group or an unrepresentable sum says what
/// they saw could not be trusted in full. Either damages the proof, so the
/// group is capped at `partial` (ruling on 34201f49). Ordinary missing usage —
/// a source that simply did not report tokens — is NOT damage and leaves
/// `complete` intact (plan D4). `none` stays `none`: damage never upgrades an
/// unobserved window.
fn effective_coverage(agg: &UsageAggregate, observed: Coverage) -> Coverage {
    let damaged = agg.rejected_counter_count > 0
        || agg.conflict_count > 0
        || agg.measured_overflow
        || agg.input_overflow
        || agg.output_overflow;
    if damaged && observed == Coverage::Complete {
        Coverage::Partial
    } else {
        observed
    }
}

fn totals_from(agg: &UsageAggregate, observed: Coverage) -> UsageTotals {
    let coverage = effective_coverage(agg, observed);
    UsageTotals {
        activity_count: agg.activity_count,
        response_count: agg.response_count,
        invocation_count: agg.invocation_count,
        measured_tokens: measured_value(agg.measured_tokens, agg.measured_overflow, agg, coverage),
        measured_event_count: agg.measured_event_count,
        unknown_usage_count: agg.unknown_usage_count,
        input_tokens: measured_component(agg.input_tokens, agg.input_overflow, agg, coverage),
        output_tokens: measured_component(agg.output_tokens, agg.output_overflow, agg, coverage),
        coverage: coverage.wire(),
    }
}

/// Component subtotals follow the same rule as the total, except that a
/// component is known whenever ANY valid event reported it — a `partial` event
/// (input known, output missing) still contributes its known side. A finite
/// component stays known even when the combined total overflowed.
fn measured_component(
    known: Option<i64>,
    overflow: bool,
    agg: &UsageAggregate,
    coverage: Coverage,
) -> Option<i64> {
    if overflow {
        return None;
    }
    if known.is_some() {
        return known;
    }
    if agg.activity_count == 0 && coverage == Coverage::Complete {
        return Some(0);
    }
    None
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// `usage.overview` — see the module docs. Every metric, breakdown and bucket
/// derives from the exact filters supplied.
pub async fn overview(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: OverviewReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    if req.days != 30 && req.days != 90 {
        return Err(AppError::Invalid(format!(
            "days must be 30 or 90, got {}",
            req.days
        )));
    }
    let tz: Tz = req
        .time_zone
        .parse()
        .map_err(|_| AppError::Invalid(format!("unknown IANA time zone: {}", req.time_zone)))?;

    let now = Utc::now();
    let buckets = build_buckets(tz, req.days, now)?;
    let range_start = buckets
        .first()
        .map(|b| b.start)
        .ok_or_else(|| AppError::Internal("empty range".into()))?;
    let range_end = now;

    // ── Filters ──────────────────────────────────────────────────────────
    let hidden = model_usage::hidden_workspace_ids(&state.db).await?;

    let workspace_sel = match req.workspace_id.as_deref() {
        None => Sel::All,
        Some(UNSCOPED_WORKSPACE_ID) => Sel::Null,
        Some(id) => {
            let row = repo::workspace::get(&state.db, id)
                .await?
                .filter(|w| !w.hidden)
                .ok_or_else(|| AppError::NotFound(format!("workspace id={id} not found")))?;
            Sel::Id(row.id)
        }
    };
    let agent_sel = match req.workspace_agent_id.as_deref() {
        None => Sel::All,
        Some(UNASSIGNED_AGENT_ID) => Sel::Null,
        Some(id) => {
            let agent = repo::workspace_agent::get(&state.db, id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("agent id={id} not found")))?;
            if hidden.contains(&agent.workspace_id) {
                return Err(AppError::NotFound(format!("agent id={id} not found")));
            }
            Sel::Id(agent.id)
        }
    };
    let model_filter = match req.model_key.as_deref() {
        None => None,
        Some(key) => Some(
            decode_key(key)
                .ok_or_else(|| AppError::Invalid(format!("malformed modelKey: {key}")))?,
        ),
    };

    let scope_query = ScopeQuery {
        workspace: workspace_sel.clone(),
        agent: agent_sel.clone(),
    };
    let scope = UsageScope {
        workspace_id: match &workspace_sel {
            Sel::Id(id) => Some(id.clone()),
            _ => None,
        },
        workspace_unscoped: workspace_sel == Sel::Null,
        workspace_agent_id: match &agent_sel {
            Sel::Id(id) => Some(id.clone()),
            _ => None,
        },
        agent_unassigned: agent_sel == Sel::Null,
        model: model_filter.as_ref().map(|m| ModelKeyFilter {
            provider: m.provider.clone(),
            name: m.name.clone(),
            basis: m.basis.wire().to_string(),
        }),
        exclude_workspace_ids: hidden.clone(),
    };
    // Options must stay discoverable, so the model dropdown is built without
    // the model filter applied to itself.
    let unfiltered_model_scope = UsageScope {
        model: None,
        ..scope.clone()
    };

    let start_text = model_usage::canonical_ts(range_start);
    let end_text = model_usage::canonical_ts(range_end);

    // ── Coverage evidence ────────────────────────────────────────────────
    let intervals = load_intervals(state, &hidden, &start_text, &end_text).await?;
    let range_coverage = coverage_for(&intervals, &scope_query, range_start, range_end);

    // ── Aggregates ───────────────────────────────────────────────────────
    let summary_agg =
        model_usage::aggregate_range(&state.db, &scope, &start_text, &end_text).await?;
    let summary = totals_from(&summary_agg, range_coverage);
    // What the scope may claim overall, after its own rows are weighed; model
    // rows and the coverage block inherit this rather than the raw observation.
    let range_coverage = effective_coverage(&summary_agg, range_coverage);

    let bucket_binds: Vec<(String, String, String)> = buckets
        .iter()
        .map(|b| {
            (
                b.date.clone(),
                model_usage::canonical_ts(b.start),
                model_usage::canonical_ts(b.end),
            )
        })
        .collect();
    let bucket_rows = model_usage::aggregate_buckets(&state.db, &scope, &bucket_binds).await?;
    let by_date: BTreeMap<String, UsageAggregate> = bucket_rows
        .into_iter()
        .map(|agg| (agg.bucket.clone(), agg))
        .collect();

    let daily: Vec<UsageDay> = buckets
        .iter()
        .map(|bucket| {
            let agg = by_date
                .get(&bucket.date)
                .cloned()
                .unwrap_or(EMPTY_AGGREGATE);
            let mut coverage = coverage_for(&intervals, &scope_query, bucket.start, bucket.end);
            // The current calendar day is still running: it can be observed so
            // far, never finished. `none` is preserved — an unobserved today is
            // not upgraded to partial.
            if bucket.in_progress && coverage == Coverage::Complete {
                coverage = Coverage::Partial;
            }
            UsageDay {
                totals: totals_from(&agg, coverage),
                date: bucket.date.clone(),
                start_utc: model_usage::canonical_ts(bucket.start),
                end_utc: model_usage::canonical_ts(bucket.end),
                in_progress: bucket.in_progress,
            }
        })
        .collect();

    // ── Breakdowns ───────────────────────────────────────────────────────
    let workspaces = workspace_options(state).await?;
    let workspace_names: BTreeMap<String, (String, bool)> = workspaces
        .iter()
        .map(|w| (w.id.clone(), (w.name.clone(), w.archived)))
        .collect();

    let agents = agent_options(state, &workspaces).await?;
    let agent_names: BTreeMap<String, (String, Option<String>)> = agents
        .iter()
        .map(|a| (a.id.clone(), (a.name.clone(), a.workspace_id.clone())))
        .collect();

    let by_workspace_raw = model_usage::aggregate_by_column(
        &state.db,
        &scope,
        GroupColumn::Workspace,
        &start_text,
        &end_text,
    )
    .await?;
    let mut by_workspace: Vec<UsageWorkspaceRow> = Vec::new();
    for (key, agg) in by_workspace_raw {
        let option = match key {
            None => UsageWorkspaceOption {
                id: UNSCOPED_WORKSPACE_ID.to_string(),
                name: UNSCOPED_WORKSPACE_NAME.to_string(),
                archived: false,
            },
            Some(id) => {
                let (name, archived) = workspace_names
                    .get(&id)
                    .cloned()
                    // A workspace deleted after its events were recorded keeps
                    // its history visible under its id rather than vanishing
                    // from a total it still contributes to.
                    .unwrap_or_else(|| (id.clone(), false));
                UsageWorkspaceOption { id, name, archived }
            }
        };
        // Breakdown rows may narrow to their own observation scope (plan D3).
        let row_scope = ScopeQuery {
            workspace: if option.id == UNSCOPED_WORKSPACE_ID {
                Sel::Null
            } else {
                Sel::Id(option.id.clone())
            },
            agent: scope_query.agent.clone(),
        };
        let coverage = coverage_for(&intervals, &row_scope, range_start, range_end);
        by_workspace.push(UsageWorkspaceRow {
            totals: totals_from(&agg, coverage),
            option,
        });
    }

    let by_agent_raw = model_usage::aggregate_by_column(
        &state.db,
        &scope,
        GroupColumn::WorkspaceAgent,
        &start_text,
        &end_text,
    )
    .await?;
    let mut by_agent: Vec<UsageAgentRow> = Vec::new();
    for (key, agg) in by_agent_raw {
        let option = match key {
            None => UsageAgentOption {
                id: UNASSIGNED_AGENT_ID.to_string(),
                name: UNASSIGNED_AGENT_NAME.to_string(),
                workspace_id: None,
            },
            Some(id) => {
                let (name, workspace_id) = agent_names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| (id.clone(), None));
                UsageAgentOption {
                    id,
                    name,
                    workspace_id,
                }
            }
        };
        let row_scope = ScopeQuery {
            workspace: scope_query.workspace.clone(),
            agent: if option.id == UNASSIGNED_AGENT_ID {
                Sel::Null
            } else {
                Sel::Id(option.id.clone())
            },
        };
        let coverage = coverage_for(&intervals, &row_scope, range_start, range_end);
        by_agent.push(UsageAgentRow {
            totals: totals_from(&agg, coverage),
            option,
        });
    }

    // Model rows inherit the ENCLOSING scope coverage (plan D3): coverage is a
    // property of the sources observed, never a per-model guarantee.
    let by_model_raw =
        model_usage::aggregate_by_model(&state.db, &scope, &start_text, &end_text).await?;
    let by_model: Vec<UsageModelRow> = by_model_raw
        .iter()
        .map(|row| {
            let identity = ModelIdentity::from_event(
                row.provider.clone(),
                row.served_model.clone(),
                row.requested_model.clone(),
            );
            UsageModelRow {
                totals: totals_from(&row.aggregate, range_coverage),
                option: identity.option(),
            }
        })
        .collect();

    // ── Contexts and model options ───────────────────────────────────────
    let contexts = contexts_for(state, &workspaces, &scope_query, model_filter.as_ref()).await?;

    let mut model_identities: BTreeSet<ModelIdentity> = BTreeSet::new();
    for row in
        model_usage::aggregate_by_model(&state.db, &unfiltered_model_scope, &start_text, &end_text)
            .await?
    {
        model_identities.insert(ModelIdentity::from_event(
            row.provider,
            row.served_model,
            row.requested_model,
        ));
    }
    for context in &contexts {
        if let Some(identity) = decode_key(&context.model_key) {
            model_identities.insert(identity);
        }
    }
    let models: Vec<UsageModelOption> = model_identities.iter().map(|m| m.option()).collect();

    // ── Coverage block ───────────────────────────────────────────────────
    let evidence = compatible(&intervals, &scope_query, range_start, range_end);
    let collecting_since = evidence
        .iter()
        .map(|row| row.start)
        .min()
        .map(model_usage::canonical_ts);
    let last_verified_at = evidence
        .iter()
        .map(|row| row.last_verified_at.clone())
        .max();
    let pending_import = pending_import_for(state, &hidden, &scope_query).await?;

    let overview = UsageOverview {
        generated_at: model_usage::canonical_ts(now),
        range: UsageRange {
            days: req.days,
            time_zone: req.time_zone.clone(),
            start_date: buckets.first().map(|b| b.date.clone()).unwrap_or_default(),
            end_date: buckets.last().map(|b| b.date.clone()).unwrap_or_default(),
            start_utc: start_text,
            end_utc: end_text,
        },
        summary,
        daily,
        models,
        agents,
        workspaces,
        by_model,
        by_agent,
        by_workspace,
        contexts,
        coverage: UsageCoverageBlock {
            state: range_coverage.wire(),
            collecting_since,
            last_verified_at,
            pending_import,
            unsupported_sources: unsupported_sources(
                &intervals,
                &scope_query,
                range_start,
                range_end,
            ),
        },
    };
    serde_json::to_value(&overview).map_err(|e| AppError::Internal(e.to_string()))
}

/// Coverage rows overlapping the window, parsed, with hidden-workspace rows
/// dropped so an unrelated scratch workspace cannot contaminate the answer.
async fn load_intervals(
    state: &AppState,
    hidden: &[String],
    start_text: &str,
    end_text: &str,
) -> Result<Vec<CoverageInterval>, AppError> {
    let rows = model_usage::coverage_overlapping(&state.db, start_text, end_text).await?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            row.workspace_id
                .as_ref()
                .map(|id| !hidden.contains(id))
                .unwrap_or(true)
        })
        .filter_map(|row| {
            let start = parse_ts(&row.interval_start)?;
            let end = parse_ts(&row.interval_end)?;
            Some(CoverageInterval {
                workspace_id: row.workspace_id,
                workspace_agent_id: row.workspace_agent_id,
                source_kind: row.source_kind,
                start,
                end,
                complete: row.state == "complete",
                unsupported: row.diagnostic_code.as_deref() == Some(UNSUPPORTED_SOURCE),
                damaged: row.state != "complete" && row.diagnostic_code.is_some(),
                last_verified_at: row.last_verified_at,
            })
        })
        .collect())
}

fn parse_ts(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Backlog is SCOPED: a cursor for another workspace (or a hidden one) says
/// nothing about the requested scope.
async fn pending_import_for(
    state: &AppState,
    hidden: &[String],
    scope: &ScopeQuery,
) -> Result<bool, AppError> {
    let cursors = model_usage::pending_cursor_scopes(&state.db).await?;
    Ok(cursors.iter().any(|cursor| {
        let not_hidden = cursor
            .workspace_id
            .as_ref()
            .map(|id| !hidden.contains(id))
            .unwrap_or(true);
        not_hidden
            && row_compatible(cursor.workspace_id.as_deref(), &scope.workspace)
            && row_compatible(cursor.workspace_agent_id.as_deref(), &scope.agent)
    }))
}

/// Every non-hidden workspace, archived included, so filters stay discoverable
/// with no events at all. `__unscoped__` is offered permanently: Library
/// drafts are a standing category, not one that appears only after activity.
async fn workspace_options(state: &AppState) -> Result<Vec<UsageWorkspaceOption>, AppError> {
    let mut options = vec![UsageWorkspaceOption {
        id: UNSCOPED_WORKSPACE_ID.to_string(),
        name: UNSCOPED_WORKSPACE_NAME.to_string(),
        archived: false,
    }];
    for row in repo::workspace::list(&state.db).await? {
        options.push(UsageWorkspaceOption {
            id: row.id,
            name: row.name,
            archived: false,
        });
    }
    for row in repo::workspace::list_archived(&state.db).await? {
        options.push(UsageWorkspaceOption {
            id: row.id,
            name: row.name,
            archived: true,
        });
    }
    Ok(options)
}

async fn agent_options(
    state: &AppState,
    workspaces: &[UsageWorkspaceOption],
) -> Result<Vec<UsageAgentOption>, AppError> {
    let definitions: BTreeMap<String, String> = repo::agent_definition::list(&state.db)
        .await?
        .into_iter()
        .map(|def| (def.id, def.name))
        .collect();
    let mut options = vec![UsageAgentOption {
        id: UNASSIGNED_AGENT_ID.to_string(),
        name: UNASSIGNED_AGENT_NAME.to_string(),
        workspace_id: None,
    }];
    for workspace in workspaces.iter().filter(|w| w.id != UNSCOPED_WORKSPACE_ID) {
        for agent in repo::workspace_agent::list_by_workspace(&state.db, &workspace.id).await? {
            let name = definitions
                .get(&agent.agent_def_id)
                .cloned()
                .unwrap_or_else(|| agent.id.clone());
            options.push(UsageAgentOption {
                id: agent.id,
                name,
                workspace_id: Some(workspace.id.clone()),
            });
        }
    }
    Ok(options)
}

/// The latest context gauge per agent — independent of the date range, but
/// filtered by the same identity filters.
///
/// The reading's own provenance is reported as observed: an unmeasured session
/// keeps `null` tokens/source rather than borrowing the current clock.
async fn contexts_for(
    state: &AppState,
    workspaces: &[UsageWorkspaceOption],
    scope: &ScopeQuery,
    model_filter: Option<&ModelIdentity>,
) -> Result<Vec<UsageContext>, AppError> {
    // Unattributed activity has no agent and therefore no context gauge.
    if scope.workspace == Sel::Null || scope.agent == Sel::Null {
        return Ok(Vec::new());
    }
    let definitions: BTreeMap<String, repo::agent_definition::AgentDefRow> =
        repo::agent_definition::list(&state.db)
            .await?
            .into_iter()
            .map(|def| (def.id.clone(), def))
            .collect();

    let mut contexts = Vec::new();
    for workspace in workspaces.iter().filter(|w| w.id != UNSCOPED_WORKSPACE_ID) {
        if let Sel::Id(want) = &scope.workspace {
            if want != &workspace.id {
                continue;
            }
        }
        for agent in repo::workspace_agent::list_by_workspace(&state.db, &workspace.id).await? {
            if let Sel::Id(want) = &scope.agent {
                if want != &agent.id {
                    continue;
                }
            }
            let def = definitions.get(&agent.agent_def_id);
            let identity = ModelIdentity {
                basis: match def.and_then(|d| d.model.clone()) {
                    // The contract: latest context uses the CONFIGURED model,
                    // always with a Selected badge — it is what we asked for,
                    // never proof of what served the tokens.
                    Some(_) => Basis::Selected,
                    None => Basis::Unknown,
                },
                provider: def.and_then(|d| {
                    provider_for_agent(d.provider_id.as_deref(), d.cli_kind.as_deref())
                }),
                name: def.and_then(|d| d.model.clone()),
            };
            let identity = if identity.basis == Basis::Unknown {
                ModelIdentity {
                    basis: Basis::Unknown,
                    provider: None,
                    name: None,
                }
            } else {
                identity
            };
            if let Some(filter) = model_filter {
                if filter != &identity {
                    continue;
                }
            }
            let session = repo::session::get_by_instance(&state.db, &agent.id).await?;
            let Some(session) = session else { continue };
            contexts.push(UsageContext {
                workspace_agent_id: agent.id.clone(),
                agent_name: def
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| agent.id.clone()),
                workspace_id: workspace.id.clone(),
                workspace_name: workspace.name.clone(),
                archived: workspace.archived,
                model_key: identity.key(),
                tokens: session.context_tokens,
                capacity: session.context_limit,
                source: session.context_source,
                observed_at: session.context_observed_at,
            });
        }
    }
    Ok(contexts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::agent_definition::AgentDefinitionInput;
    use crate::engine::repo::model_usage::{
        insert_coverage, insert_event, CoverageIntervalRow, NewUsageEvent,
    };
    use crate::engine::repo::{agent_definition, session, workspace, workspace_agent};
    use chrono::Timelike;
    use serde_json::json;
    use sqlx::SqlitePool;

    // ── Fixtures ─────────────────────────────────────────────────────────

    fn ts(at: DateTime<Utc>) -> String {
        model_usage::canonical_ts(at)
    }

    /// A valid, minimal event: one Claude response, unscoped, no usage
    /// observed. Tests set only the fields they are actually about.
    fn ev(key: &str, at: DateTime<Utc>) -> NewUsageEvent {
        NewUsageEvent {
            id: format!("id-{key}"),
            event_key: key.into(),
            workspace_id: None,
            workspace_agent_id: None,
            session_id: None,
            generation: None,
            source_kind: SOURCE_CLAUDE_TRANSCRIPT.into(),
            source_version: "v1".into(),
            event_kind: "response".into(),
            source_session_id: None,
            source_request_id: None,
            source_response_id: None,
            occurred_at: at,
            recorded_at: at,
            provider: None,
            requested_model: None,
            served_model: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_input_tokens: None,
            cache_write_input_tokens: None,
            reasoning_output_tokens: None,
            validity: "valid".into(),
            diagnostic_code: None,
        }
    }

    /// Observation of EVERY collected source over one interval — the only way
    /// a window can legitimately reach `complete`.
    async fn cover_all_sources(
        pool: &SqlitePool,
        workspace_id: Option<&str>,
        agent_id: Option<&str>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        state: &str,
    ) {
        for source in COLLECTED_SOURCES {
            insert_coverage(
                pool,
                &CoverageIntervalRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    workspace_id: workspace_id.map(str::to_string),
                    workspace_agent_id: agent_id.map(str::to_string),
                    source_kind: (*source).to_string(),
                    interval_start: ts(start),
                    interval_end: ts(end),
                    state: state.into(),
                    collector_version: "v1".into(),
                    diagnostic_code: None,
                    last_verified_at: ts(end),
                },
            )
            .await
            .expect("insert coverage failed");
        }
    }

    /// Global coverage of everything this query could ask about.
    async fn cover_everything(pool: &SqlitePool, state: &str) {
        let now = Utc::now();
        cover_all_sources(
            pool,
            None,
            None,
            now - Duration::days(400),
            now + Duration::days(1),
            state,
        )
        .await;
    }

    async fn fixture_agent(
        state: &AppState,
        workspace_id: &str,
        name: &str,
        model: Option<&str>,
        cli_kind: Option<&str>,
    ) -> String {
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: name.into(),
                role: None,
                agent_type: "cli".into(),
                cli_kind: cli_kind.map(str::to_string),
                color: None,
                provider_id: None,
                model: model.map(str::to_string),
                harness_mode: "own".into(),
                share_blackboard: None,
                auto_submit_injected: None,
                allowed_senders: None,
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::instantiate(&state.db, workspace_id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    async fn overview_of(state: &AppState, req: Value) -> Value {
        overview(state, req).await.expect("overview failed")
    }

    fn i64_at(value: &Value, path: &str) -> i64 {
        value
            .pointer(path)
            .and_then(Value::as_i64)
            .unwrap_or_else(|| {
                panic!(
                    "expected an integer at {path}, got {:?}",
                    value.pointer(path)
                )
            })
    }

    fn rows<'a>(value: &'a Value, key: &str) -> &'a Vec<Value> {
        value
            .get(key)
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{key} must be an array"))
    }

    fn row_by<'a>(value: &'a Value, key: &str, id_field: &str, id: &str) -> &'a Value {
        rows(value, key)
            .iter()
            .find(|row| row.get(id_field).and_then(Value::as_str) == Some(id))
            .unwrap_or_else(|| panic!("no {key} row with {id_field}={id}"))
    }

    // ── Model identity ───────────────────────────────────────────────────

    /// A key must survive a round trip even when the model name contains the
    /// separator, or a crafted name could impersonate another identity.
    #[test]
    fn model_key_round_trips_through_separators() {
        for identity in [
            ModelIdentity {
                basis: Basis::Reported,
                provider: Some("anthropic".into()),
                name: Some("claude-opus-5".into()),
            },
            ModelIdentity {
                basis: Basis::Selected,
                provider: None,
                name: Some("weird:name\\with:escapes".into()),
            },
            ModelIdentity {
                basis: Basis::Selected,
                provider: Some("".into()),
                name: Some("empty-provider".into()),
            },
            ModelIdentity {
                basis: Basis::Unknown,
                provider: None,
                name: None,
            },
        ] {
            let key = identity.key();
            assert_eq!(
                decode_key(&key).as_ref(),
                Some(&identity),
                "key {key} did not round-trip"
            );
        }
    }

    /// Reported and Selected are different identities for the same name — the
    /// whole point of the basis (contract: no unlabelled substitution).
    #[test]
    fn reported_and_selected_keys_never_collide() {
        let reported = ModelIdentity {
            basis: Basis::Reported,
            provider: Some("anthropic".into()),
            name: Some("claude-opus-5".into()),
        };
        let selected = ModelIdentity {
            basis: Basis::Selected,
            ..reported.clone()
        };
        assert_ne!(reported.key(), selected.key());
    }

    #[test]
    fn malformed_model_keys_are_rejected() {
        for key in [
            "garbage",
            "reported:=anthropic",             // too few components
            "bogus:=a:=b",                     // unknown basis
            "unknown:=anthropic:=name",        // unknown identity carries none
            "reported:=anthropic:",            // reported needs a name
            "reported:=anthropic:=name:extra", // too many components
            "reported:=a:=b\\",                // dangling escape
        ] {
            assert!(decode_key(key).is_none(), "{key} must be rejected");
        }
    }

    // ── Calendar buckets ─────────────────────────────────────────────────

    #[test]
    fn buckets_are_n_trailing_dates_ending_today() {
        let tz: Tz = "UTC".parse().unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 5, 14, 30, 0).unwrap();
        let buckets = build_buckets(tz, 30, now).unwrap();
        assert_eq!(buckets.len(), 30);
        assert_eq!(buckets[0].date, "2026-08-07");
        assert_eq!(buckets[29].date, "2026-09-05");
        assert!(buckets[29].in_progress);
        assert!(!buckets[28].in_progress);
        // Today ends NOW, not at tomorrow's midnight.
        assert_eq!(buckets[29].end, now);
        // Half-open and contiguous: each bucket starts where the last ended.
        for pair in buckets.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "gap between daily buckets");
        }
    }

    /// A spring-forward day is 23 hours and a fall-back day is 25 — proof the
    /// bounds come from a real IANA calendar, not from `date * 86400`.
    #[test]
    fn dst_days_are_23_and_25_hours() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let spring = local_day_start(tz, NaiveDate::from_ymd_opt(2026, 3, 8).unwrap()).unwrap();
        let after_spring =
            local_day_start(tz, NaiveDate::from_ymd_opt(2026, 3, 9).unwrap()).unwrap();
        assert_eq!((after_spring - spring).num_hours(), 23);

        let fall = local_day_start(tz, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()).unwrap();
        let after_fall =
            local_day_start(tz, NaiveDate::from_ymd_opt(2026, 11, 2).unwrap()).unwrap();
        assert_eq!((after_fall - fall).num_hours(), 25);
    }

    /// Havana moves its clock AT midnight in spring: local 00:00 does not
    /// exist, and the day must start at the transition instant instead of
    /// failing or silently landing on the previous day.
    #[test]
    fn a_skipped_local_midnight_starts_at_the_transition() {
        let tz: Tz = "America/Havana".parse().unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let start = local_day_start(tz, date).unwrap();
        let local = start.with_timezone(&tz);
        assert_eq!(local.date_naive(), date, "must stay on the requested date");
        assert_eq!(local.hour(), 1, "clocks jump 00:00 -> 01:00");
    }

    // ── Coverage lattice ─────────────────────────────────────────────────

    fn interval(
        workspace_id: Option<&str>,
        agent_id: Option<&str>,
        source: &str,
        start_h: i64,
        end_h: i64,
        complete: bool,
    ) -> CoverageInterval {
        let base = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        CoverageInterval {
            workspace_id: workspace_id.map(str::to_string),
            workspace_agent_id: agent_id.map(str::to_string),
            source_kind: source.into(),
            start: base + Duration::hours(start_h),
            end: base + Duration::hours(end_h),
            complete,
            unsupported: false,
            damaged: false,
            last_verified_at: "2026-09-02T00:00:00.000Z".into(),
        }
    }

    fn all_sources(workspace_id: Option<&str>, agent_id: Option<&str>) -> Vec<CoverageInterval> {
        COLLECTED_SOURCES
            .iter()
            .map(|s| interval(workspace_id, agent_id, s, 0, 24, true))
            .collect()
    }

    fn window() -> (DateTime<Utc>, DateTime<Utc>) {
        let base = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        (base, base + Duration::hours(24))
    }

    #[test]
    fn no_overlapping_interval_is_none_not_partial() {
        let (start, end) = window();
        let scope = ScopeQuery {
            workspace: Sel::All,
            agent: Sel::All,
        };
        assert_eq!(coverage_for(&[], &scope, start, end), Coverage::None);
        let elsewhere = vec![interval(None, None, SOURCE_DRAFT, 48, 72, true)];
        assert_eq!(
            coverage_for(&elsewhere, &scope, start, end),
            Coverage::None,
            "an interval outside the window proves nothing about it"
        );
    }

    /// A collector that only ever saw one workspace cannot certify the whole
    /// account, but the same rows DO prove that workspace's own window.
    #[test]
    fn a_narrower_scope_cannot_prove_a_broader_query() {
        let (start, end) = window();
        let intervals = all_sources(Some("ws-1"), None);
        let global = ScopeQuery {
            workspace: Sel::All,
            agent: Sel::All,
        };
        assert_eq!(
            coverage_for(&intervals, &global, start, end),
            Coverage::Partial
        );
        let scoped = ScopeQuery {
            workspace: Sel::Id("ws-1".into()),
            agent: Sel::All,
        };
        assert_eq!(
            coverage_for(&intervals, &scoped, start, end),
            Coverage::Complete
        );
    }

    /// A global (NULL-scope) observation proves any narrower question.
    #[test]
    fn a_global_scope_proves_a_narrower_query() {
        let (start, end) = window();
        let intervals = all_sources(None, None);
        let scoped = ScopeQuery {
            workspace: Sel::Id("ws-1".into()),
            agent: Sel::Id("agent-1".into()),
        };
        assert_eq!(
            coverage_for(&intervals, &scoped, start, end),
            Coverage::Complete
        );
    }

    /// Complete needs EVERY collected source, not merely the sources that
    /// happen to have rows.
    #[test]
    fn a_missing_source_keeps_the_window_partial() {
        let (start, end) = window();
        let mut intervals = all_sources(None, None);
        intervals.retain(|row| row.source_kind != SOURCE_CODEX_TRANSCRIPT);
        let scope = ScopeQuery {
            workspace: Sel::All,
            agent: Sel::All,
        };
        assert_eq!(
            coverage_for(&intervals, &scope, start, end),
            Coverage::Partial
        );
    }

    #[test]
    fn a_gap_inside_one_source_keeps_the_window_partial() {
        let (start, end) = window();
        let mut intervals: Vec<CoverageInterval> = COLLECTED_SOURCES
            .iter()
            .filter(|s| **s != SOURCE_DRAFT)
            .map(|s| interval(None, None, s, 0, 24, true))
            .collect();
        // Draft observed 0-10 and 12-24: two hours nobody watched.
        intervals.push(interval(None, None, SOURCE_DRAFT, 0, 10, true));
        intervals.push(interval(None, None, SOURCE_DRAFT, 12, 24, true));
        let scope = ScopeQuery {
            workspace: Sel::All,
            agent: Sel::All,
        };
        assert_eq!(
            coverage_for(&intervals, &scope, start, end),
            Coverage::Partial
        );
    }

    #[test]
    fn adjacent_intervals_close_the_window() {
        let (start, end) = window();
        let mut intervals: Vec<CoverageInterval> = COLLECTED_SOURCES
            .iter()
            .filter(|s| **s != SOURCE_DRAFT)
            .map(|s| interval(None, None, s, 0, 24, true))
            .collect();
        intervals.push(interval(None, None, SOURCE_DRAFT, 0, 10, true));
        intervals.push(interval(None, None, SOURCE_DRAFT, 10, 24, true));
        let scope = ScopeQuery {
            workspace: Sel::All,
            agent: Sel::All,
        };
        assert_eq!(
            coverage_for(&intervals, &scope, start, end),
            Coverage::Complete
        );
    }

    #[test]
    fn an_unsupported_source_prevents_complete() {
        let (start, end) = window();
        let mut intervals = all_sources(None, None);
        let mut unsupported = interval(None, None, "codex-legacy", 0, 24, false);
        unsupported.unsupported = true;
        intervals.push(unsupported);
        let scope = ScopeQuery {
            workspace: Sel::All,
            agent: Sel::All,
        };
        assert_eq!(
            coverage_for(&intervals, &scope, start, end),
            Coverage::Partial
        );
        assert_eq!(
            unsupported_sources(&intervals, &scope, start, end),
            vec!["codex-legacy".to_string()]
        );
    }

    /// An unsupported source seen only in another workspace must not
    /// contaminate an unrelated scoped query.
    #[test]
    fn unsupported_diagnostics_are_scoped() {
        let (start, end) = window();
        let mut intervals = all_sources(None, None);
        let mut unsupported = interval(Some("ws-other"), None, "codex-legacy", 0, 24, false);
        unsupported.unsupported = true;
        intervals.push(unsupported);
        let mine = ScopeQuery {
            workspace: Sel::Id("ws-mine".into()),
            agent: Sel::All,
        };
        assert!(unsupported_sources(&intervals, &mine, start, end).is_empty());
        assert_eq!(
            coverage_for(&intervals, &mine, start, end),
            Coverage::Complete
        );
    }

    // ── D4: measured vs unknown ──────────────────────────────────────────

    #[test]
    fn measured_tokens_follow_the_d4_table() {
        let measured = UsageAggregate {
            activity_count: 2,
            measured_event_count: 2,
            measured_tokens: Some(0),
            ..EMPTY_AGGREGATE
        };
        assert_eq!(
            measured_value(
                measured.measured_tokens,
                false,
                &measured,
                Coverage::Partial
            ),
            Some(0),
            "a source-reported zero IS a measurement, whatever the coverage"
        );

        let empty = EMPTY_AGGREGATE;
        assert_eq!(
            measured_value(None, false, &empty, Coverage::Complete),
            Some(0),
            "verified empty window"
        );
        assert_eq!(measured_value(None, false, &empty, Coverage::Partial), None);
        assert_eq!(measured_value(None, false, &empty, Coverage::None), None);

        let unknown_only = UsageAggregate {
            activity_count: 3,
            unknown_usage_count: 3,
            ..EMPTY_AGGREGATE
        };
        assert_eq!(
            measured_value(None, false, &unknown_only, Coverage::Complete),
            None,
            "complete observation never turns missing usage into zero"
        );

        // An unrepresentable sum is unavailable even though events were
        // measured — never 0, never rounded — and damages the proof.
        let overflowed = UsageAggregate {
            activity_count: 5,
            measured_event_count: 5,
            measured_overflow: true,
            ..EMPTY_AGGREGATE
        };
        assert_eq!(
            measured_value(None, true, &overflowed, Coverage::Complete),
            None
        );
        assert_eq!(
            effective_coverage(&overflowed, Coverage::Complete),
            Coverage::Partial
        );
        assert_eq!(
            effective_coverage(&overflowed, Coverage::None),
            Coverage::None,
            "damage never upgrades an unobserved window"
        );
    }

    /// Ruling on 34201f49: a rejected counter or a conflict inside a fully
    /// observed window caps that window at partial, while an ordinary
    /// unknown-usage event leaves `complete` intact (D4 preserved).
    #[tokio::test]
    async fn damaged_events_cap_complete_coverage_but_plain_unknowns_do_not() {
        let state = AppState::for_tests().await;
        cover_everything(&state.db, "complete").await;
        let at = Utc::now() - Duration::hours(1);

        // Plain unknown usage: still complete.
        insert_event(&state.db, &ev("blind", at)).await.unwrap();
        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(out.pointer("/summary/coverage").unwrap(), "complete");
        assert_eq!(out.pointer("/coverage/state").unwrap(), "complete");

        // A rejected counter: the observation was damaged — even when the
        // collector had stamped its own diagnostic on the row.
        let mut rejected = ev("rejected", at);
        rejected.input_tokens = Some(-1);
        rejected.output_tokens = Some(10);
        rejected.diagnostic_code = Some("collector_note".into());
        insert_event(&state.db, &rejected).await.unwrap();
        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(out.pointer("/summary/coverage").unwrap(), "partial");
        assert_eq!(out.pointer("/coverage/state").unwrap(), "partial");
        let damaged_date = at.format("%Y-%m-%d").to_string();
        assert_eq!(
            row_by(&out, "daily", "date", &damaged_date)
                .get("coverage")
                .unwrap(),
            "partial",
            "the day holding the damaged row is capped"
        );
        assert_eq!(rows(&out, "byModel")[0].get("coverage").unwrap(), "partial");
        assert_eq!(
            i64_at(&out, "/summary/outputTokens"),
            10,
            "the finite known component is preserved"
        );
        // A day that held no damaged row keeps its proof: the oldest bucket is
        // 29 days before `at`, so it can never be the damaged date.
        assert_eq!(rows(&out, "daily")[0].get("coverage").unwrap(), "complete");
    }

    #[tokio::test]
    async fn a_conflicting_response_caps_complete_coverage() {
        let state = AppState::for_tests().await;
        cover_everything(&state.db, "complete").await;
        let at = Utc::now() - Duration::hours(1);
        let mut e = ev("dup", at);
        e.input_tokens = Some(5);
        e.output_tokens = Some(5);
        insert_event(&state.db, &e).await.unwrap();
        model_usage::mark_conflict(&state.db, "dup", "claude_group_disagrees")
            .await
            .unwrap();
        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(
            i64_at(&out, "/summary/activityCount"),
            1,
            "still one activity"
        );
        assert!(out.pointer("/summary/measuredTokens").unwrap().is_null());
        assert_eq!(out.pointer("/summary/coverage").unwrap(), "partial");
    }

    /// A compatible partial interval that carries a diagnostic caps complete
    /// proof even when complete spans cover the same window.
    #[test]
    fn a_damaged_partial_interval_caps_complete_proof() {
        let (start, end) = window();
        let mut intervals = all_sources(None, None);
        let mut damaged = interval(Some("ws-1"), None, SOURCE_CLAUDE_TRANSCRIPT, 0, 24, false);
        damaged.damaged = true;
        intervals.push(damaged);
        let global = ScopeQuery {
            workspace: Sel::All,
            agent: Sel::All,
        };
        assert_eq!(
            coverage_for(&intervals, &global, start, end),
            Coverage::Partial
        );
        // A scope the damaged row is not compatible with keeps its proof.
        let other = ScopeQuery {
            workspace: Sel::Id("ws-2".into()),
            agent: Sel::All,
        };
        assert_eq!(
            coverage_for(&intervals, &other, start, end),
            Coverage::Complete
        );
    }

    // ── Handler: validation ──────────────────────────────────────────────

    #[tokio::test]
    async fn rejects_bad_days_and_time_zones() {
        let state = AppState::for_tests().await;
        for days in [0, 7, 45, 91] {
            let err = overview(&state, json!({ "days": days, "timeZone": "UTC" }))
                .await
                .expect_err("bad day count must be rejected");
            assert!(matches!(err, AppError::Invalid(_)), "got {err:?}");
        }
        let err = overview(&state, json!({ "days": 30, "timeZone": "Mars/Olympus" }))
            .await
            .expect_err("unknown zone must be rejected");
        assert!(matches!(err, AppError::Invalid(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn rejects_unknown_ids_and_malformed_model_keys() {
        let state = AppState::for_tests().await;
        let err = overview(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "workspaceId": "nope" }),
        )
        .await
        .expect_err("unknown workspace must be NotFound");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");

        let err = overview(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "workspaceAgentId": "nope" }),
        )
        .await
        .expect_err("unknown agent must be NotFound");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");

        let err = overview(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "modelKey": "garbage" }),
        )
        .await
        .expect_err("malformed key must be Invalid");
        assert!(matches!(err, AppError::Invalid(_)), "got {err:?}");
    }

    /// A well-formed key nobody ever used is a legitimate empty answer, not an
    /// error.
    #[tokio::test]
    async fn an_unused_model_key_returns_an_empty_result() {
        let state = AppState::for_tests().await;
        insert_event(&state.db, &ev("k1", Utc::now() - Duration::hours(2)))
            .await
            .unwrap();
        let out = overview_of(
            &state,
            json!({
                "days": 30,
                "timeZone": "UTC",
                "modelKey": "reported:=anthropic:=ghost-model",
            }),
        )
        .await;
        assert_eq!(i64_at(&out, "/summary/activityCount"), 0);
        assert!(rows(&out, "byModel").is_empty());
    }

    // ── Handler: shape ───────────────────────────────────────────────────

    #[tokio::test]
    async fn an_empty_database_is_honestly_unavailable() {
        let state = AppState::for_tests().await;
        let out = overview_of(&state, json!({ "days": 90, "timeZone": "UTC" })).await;

        assert_eq!(rows(&out, "daily").len(), 90, "exactly N ascending rows");
        assert_eq!(i64_at(&out, "/summary/activityCount"), 0);
        assert!(
            out.pointer("/summary/measuredTokens").unwrap().is_null(),
            "never a green zero without observation"
        );
        assert_eq!(
            out.pointer("/summary/coverage").unwrap().as_str(),
            Some("none")
        );
        assert_eq!(
            out.pointer("/coverage/state").unwrap().as_str(),
            Some("none")
        );
        assert!(out.pointer("/coverage/collectingSince").unwrap().is_null());
        assert_eq!(
            out.pointer("/coverage/pendingImport").unwrap().as_bool(),
            Some(false)
        );
        for day in rows(&out, "daily") {
            assert_eq!(day.get("coverage").unwrap().as_str(), Some("none"));
            assert!(day.get("measuredTokens").unwrap().is_null());
        }
        let last = rows(&out, "daily").last().unwrap();
        assert_eq!(last.get("inProgress").unwrap().as_bool(), Some(true));
    }

    /// Full observation with no activity is the one case that may report 0 —
    /// and today's unfinished row still may not claim `complete`.
    #[tokio::test]
    async fn a_fully_observed_empty_window_reports_zero_but_today_stays_partial() {
        let state = AppState::for_tests().await;
        cover_everything(&state.db, "complete").await;

        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(
            out.pointer("/summary/coverage").unwrap().as_str(),
            Some("complete")
        );
        assert_eq!(i64_at(&out, "/summary/measuredTokens"), 0);
        assert_eq!(i64_at(&out, "/summary/inputTokens"), 0);
        assert_eq!(i64_at(&out, "/summary/outputTokens"), 0);

        let daily = rows(&out, "daily");
        let today = daily.last().unwrap();
        assert_eq!(today.get("inProgress").unwrap().as_bool(), Some(true));
        assert_eq!(
            today.get("coverage").unwrap().as_str(),
            Some("partial"),
            "an unfinished calendar day is never complete"
        );
        assert!(
            today.get("measuredTokens").unwrap().is_null(),
            "partial + empty is unknown, not zero"
        );
        let yesterday = &daily[daily.len() - 2];
        assert_eq!(
            yesterday.get("coverage").unwrap().as_str(),
            Some("complete")
        );
        assert_eq!(yesterday.get("measuredTokens").unwrap().as_i64(), Some(0));
    }

    /// Partial observation of an empty window stays unknown.
    #[tokio::test]
    async fn a_partially_observed_empty_window_is_unknown() {
        let state = AppState::for_tests().await;
        cover_everything(&state.db, "partial").await;
        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(
            out.pointer("/summary/coverage").unwrap().as_str(),
            Some("partial")
        );
        assert!(out.pointer("/summary/measuredTokens").unwrap().is_null());
    }

    /// Today with no observation at all keeps `none` — it is not upgraded to
    /// partial just because it is in progress.
    #[tokio::test]
    async fn today_without_observation_stays_none() {
        let state = AppState::for_tests().await;
        let now = Utc::now();
        cover_all_sources(
            &state.db,
            None,
            None,
            now - Duration::days(400),
            now - Duration::days(2),
            "complete",
        )
        .await;
        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        let daily = rows(&out, "daily");
        assert_eq!(daily.last().unwrap().get("coverage").unwrap(), "none");
        assert_eq!(
            out.pointer("/summary/coverage").unwrap().as_str(),
            Some("partial"),
            "the range as a whole was observed, but not to its end"
        );
    }

    #[tokio::test]
    async fn measured_and_unknown_events_are_reported_separately() {
        let state = AppState::for_tests().await;
        cover_everything(&state.db, "complete").await;
        let at = Utc::now() - Duration::hours(3);

        let mut known = ev("known", at);
        known.input_tokens = Some(1_200);
        known.output_tokens = Some(300);
        insert_event(&state.db, &known).await.unwrap();

        // Input observed, output missing: one KNOWN component, no measurable
        // total.
        let mut half = ev("half", at);
        half.input_tokens = Some(50);
        insert_event(&state.db, &half).await.unwrap();

        // Nothing observed at all.
        insert_event(&state.db, &ev("blind", at)).await.unwrap();

        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(i64_at(&out, "/summary/activityCount"), 3);
        assert_eq!(i64_at(&out, "/summary/measuredEventCount"), 1);
        assert_eq!(i64_at(&out, "/summary/unknownUsageCount"), 2);
        assert_eq!(i64_at(&out, "/summary/measuredTokens"), 1_500);
        assert_eq!(i64_at(&out, "/summary/inputTokens"), 1_250);
        assert_eq!(i64_at(&out, "/summary/outputTokens"), 300);
    }

    /// Complete observation plus events whose tokens were never observed is
    /// still `null` — coverage proves the events, not their usage.
    #[tokio::test]
    async fn complete_coverage_with_unknown_events_is_still_null() {
        let state = AppState::for_tests().await;
        cover_everything(&state.db, "complete").await;
        insert_event(&state.db, &ev("blind", Utc::now() - Duration::hours(1)))
            .await
            .unwrap();
        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(
            out.pointer("/summary/coverage").unwrap().as_str(),
            Some("complete")
        );
        assert!(out.pointer("/summary/measuredTokens").unwrap().is_null());
        assert_eq!(i64_at(&out, "/summary/unknownUsageCount"), 1);
    }

    // ── D1: unscoped and unassigned ──────────────────────────────────────

    #[tokio::test]
    async fn unscoped_and_unassigned_buckets_reconcile_to_the_summary() {
        let state = AppState::for_tests().await;
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .unwrap()
            .id;
        let agent = fixture_agent(
            &state,
            &ws,
            "Dew",
            Some("claude-opus-5"),
            Some("claude-code"),
        )
        .await;
        let at = Utc::now() - Duration::hours(2);

        // A Library draft: no workspace, no agent.
        let mut draft = ev("draft", at);
        draft.event_kind = "invocation".into();
        draft.source_kind = SOURCE_DRAFT.into();
        insert_event(&state.db, &draft).await.unwrap();

        // A workspace draft with no agent attribution.
        let mut ws_draft = ev("ws-draft", at);
        ws_draft.event_kind = "invocation".into();
        ws_draft.source_kind = SOURCE_DRAFT.into();
        ws_draft.workspace_id = Some(ws.clone());
        insert_event(&state.db, &ws_draft).await.unwrap();

        // A normal attributed response.
        let mut attributed = ev("attributed", at);
        attributed.workspace_id = Some(ws.clone());
        attributed.workspace_agent_id = Some(agent.clone());
        insert_event(&state.db, &attributed).await.unwrap();

        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(i64_at(&out, "/summary/activityCount"), 3);
        assert_eq!(i64_at(&out, "/summary/responseCount"), 1);
        assert_eq!(i64_at(&out, "/summary/invocationCount"), 2);

        let unscoped = row_by(&out, "byWorkspace", "id", UNSCOPED_WORKSPACE_ID);
        assert_eq!(unscoped.get("name").unwrap(), "No workspace");
        assert_eq!(unscoped.get("activityCount").unwrap().as_i64(), Some(1));
        assert_eq!(
            row_by(&out, "byWorkspace", "id", &ws)
                .get("activityCount")
                .unwrap()
                .as_i64(),
            Some(2)
        );
        let unassigned = row_by(&out, "byAgent", "id", UNASSIGNED_AGENT_ID);
        assert_eq!(unassigned.get("activityCount").unwrap().as_i64(), Some(2));
        assert!(unassigned.get("workspaceId").unwrap().is_null());
        assert_eq!(
            row_by(&out, "byAgent", "id", &agent)
                .get("activityCount")
                .unwrap()
                .as_i64(),
            Some(1)
        );

        // Every grouping reconciles to the summary.
        for key in ["byWorkspace", "byAgent", "byModel"] {
            let total: i64 = rows(&out, key)
                .iter()
                .map(|row| row.get("activityCount").unwrap().as_i64().unwrap())
                .sum();
            assert_eq!(total, 3, "{key} must reconcile to the summary total");
        }

        // A real workspace filter excludes the unscoped record.
        let scoped = overview_of(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "workspaceId": ws }),
        )
        .await;
        assert_eq!(i64_at(&scoped, "/summary/activityCount"), 2);

        // The unscoped filter selects exactly the record with no workspace.
        let unscoped_only = overview_of(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "workspaceId": UNSCOPED_WORKSPACE_ID }),
        )
        .await;
        assert_eq!(i64_at(&unscoped_only, "/summary/activityCount"), 1);
        assert!(
            rows(&unscoped_only, "contexts").is_empty(),
            "unattributed activity has no context gauge"
        );

        // The unassigned filter means "no workspace agent", in any workspace.
        let unassigned_only = overview_of(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "workspaceAgentId": UNASSIGNED_AGENT_ID }),
        )
        .await;
        assert_eq!(i64_at(&unassigned_only, "/summary/activityCount"), 2);
    }

    /// An agent that does not belong to the selected workspace returns an
    /// empty scoped result — never someone else's rows.
    #[tokio::test]
    async fn a_mismatched_agent_and_workspace_return_nothing() {
        let state = AppState::for_tests().await;
        let ws_a = workspace::create(&state.db, "A", "/tmp/a", None)
            .await
            .unwrap()
            .id;
        let ws_b = workspace::create(&state.db, "B", "/tmp/b", None)
            .await
            .unwrap()
            .id;
        let agent = fixture_agent(&state, &ws_a, "Dew", None, None).await;
        let mut event = ev("e", Utc::now() - Duration::hours(1));
        event.workspace_id = Some(ws_a.clone());
        event.workspace_agent_id = Some(agent.clone());
        insert_event(&state.db, &event).await.unwrap();

        let out = overview_of(
            &state,
            json!({
                "days": 30,
                "timeZone": "UTC",
                "workspaceId": ws_b,
                "workspaceAgentId": agent,
            }),
        )
        .await;
        assert_eq!(i64_at(&out, "/summary/activityCount"), 0);
        assert!(rows(&out, "byAgent").is_empty());
        assert!(rows(&out, "contexts").is_empty());
    }

    // ── Model basis ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn reported_and_selected_models_stay_separate_rows_and_filters() {
        let state = AppState::for_tests().await;
        let at = Utc::now() - Duration::hours(4);

        let mut served = ev("served", at);
        served.provider = Some("anthropic".into());
        served.requested_model = Some("claude-opus-5".into());
        served.served_model = Some("claude-opus-5".into());
        served.input_tokens = Some(10);
        served.output_tokens = Some(5);
        insert_event(&state.db, &served).await.unwrap();

        let mut requested_only = ev("requested", at);
        requested_only.provider = Some("anthropic".into());
        requested_only.requested_model = Some("claude-opus-5".into());
        insert_event(&state.db, &requested_only).await.unwrap();

        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        let by_model = rows(&out, "byModel");
        assert_eq!(by_model.len(), 2, "same name, two identities");
        let bases: BTreeSet<&str> = by_model
            .iter()
            .map(|row| row.get("basis").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(
            bases,
            BTreeSet::from(["reported", "selected"]),
            "a requested model must never be labelled reported"
        );

        let reported_key = ModelIdentity {
            basis: Basis::Reported,
            provider: Some("anthropic".into()),
            name: Some("claude-opus-5".into()),
        }
        .key();
        let filtered = overview_of(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "modelKey": reported_key }),
        )
        .await;
        assert_eq!(i64_at(&filtered, "/summary/activityCount"), 1);
        assert_eq!(i64_at(&filtered, "/summary/measuredTokens"), 15);
        assert_eq!(rows(&filtered, "byModel").len(), 1);
        assert!(
            rows(&filtered, "models").len() >= 2,
            "options stay discoverable under a model filter"
        );
    }

    #[tokio::test]
    async fn an_event_with_no_model_at_all_is_one_unknown_identity() {
        let state = AppState::for_tests().await;
        let at = Utc::now() - Duration::hours(1);
        insert_event(&state.db, &ev("a", at)).await.unwrap();
        insert_event(&state.db, &ev("b", at)).await.unwrap();
        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        let by_model = rows(&out, "byModel");
        assert_eq!(by_model.len(), 1);
        assert_eq!(by_model[0].get("basis").unwrap(), "unknown");
        assert_eq!(by_model[0].get("name").unwrap(), UNKNOWN_MODEL_NAME);
        assert!(by_model[0].get("provider").unwrap().is_null());
        assert_eq!(by_model[0].get("activityCount").unwrap().as_i64(), Some(2));
    }

    // ── Hidden workspaces ────────────────────────────────────────────────

    #[tokio::test]
    async fn hidden_workspaces_are_outside_usage_scope() {
        let state = AppState::for_tests().await;
        let hidden = workspace::create_hidden(&state.db, "scratch", "/tmp/scratch")
            .await
            .unwrap()
            .id;
        let visible = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .unwrap()
            .id;
        let at = Utc::now() - Duration::hours(1);

        let mut scratch_event = ev("scratch", at);
        scratch_event.workspace_id = Some(hidden.clone());
        insert_event(&state.db, &scratch_event).await.unwrap();

        let mut real_event = ev("real", at);
        real_event.workspace_id = Some(visible.clone());
        insert_event(&state.db, &real_event).await.unwrap();

        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(
            i64_at(&out, "/summary/activityCount"),
            1,
            "the scratch workspace's activity is not the user's usage"
        );
        assert!(rows(&out, "byWorkspace")
            .iter()
            .all(|row| row.get("id").unwrap() != hidden.as_str()));
        assert!(rows(&out, "workspaces")
            .iter()
            .all(|row| row.get("id").unwrap() != hidden.as_str()));

        let err = overview(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "workspaceId": hidden }),
        )
        .await
        .expect_err("a hidden workspace is not a usage filter");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }

    // ── Options and contexts ─────────────────────────────────────────────

    /// Filters must be discoverable before any event exists, archived
    /// workspaces included.
    #[tokio::test]
    async fn options_list_every_workspace_and_agent_without_events() {
        let state = AppState::for_tests().await;
        let live = workspace::create(&state.db, "Live", "/tmp/live", None)
            .await
            .unwrap()
            .id;
        let archived = workspace::create(&state.db, "Archived", "/tmp/arch", None)
            .await
            .unwrap()
            .id;
        workspace::set_archived(&state.db, &archived, Some("2026-09-01T00:00:00Z"))
            .await
            .unwrap();
        let agent = fixture_agent(&state, &live, "Dew", None, None).await;

        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(
            row_by(&out, "workspaces", "id", &archived)
                .get("archived")
                .unwrap()
                .as_bool(),
            Some(true),
            "archived history stays reachable and labelled"
        );
        assert_eq!(
            row_by(&out, "workspaces", "id", &live)
                .get("archived")
                .unwrap()
                .as_bool(),
            Some(false)
        );
        row_by(&out, "workspaces", "id", UNSCOPED_WORKSPACE_ID);
        row_by(&out, "agents", "id", UNASSIGNED_AGENT_ID);
        assert_eq!(
            row_by(&out, "agents", "id", &agent).get("name").unwrap(),
            "Dew"
        );
    }

    /// The context gauge is the latest reading with its own provenance —
    /// independent of the date range, never re-dated by the query.
    #[tokio::test]
    async fn contexts_are_latest_per_agent_with_real_provenance() {
        let state = AppState::for_tests().await;
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .unwrap()
            .id;
        let agent = fixture_agent(
            &state,
            &ws,
            "Dew",
            Some("claude-opus-5"),
            Some("claude-code"),
        )
        .await;
        let other = fixture_agent(&state, &ws, "Mellow", None, None).await;
        let session = session::get_by_instance(&state.db, &agent)
            .await
            .unwrap()
            .expect("session exists");
        session::set_context_reading_observed(
            &state.db,
            &session.id,
            120_000,
            1_000_000,
            "claude-code",
            "2026-09-05T07:00:00.000Z",
        )
        .await
        .unwrap();

        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        let context = row_by(&out, "contexts", "workspaceAgentId", &agent);
        assert_eq!(context.get("tokens").unwrap().as_i64(), Some(120_000));
        assert_eq!(context.get("capacity").unwrap().as_i64(), Some(1_000_000));
        assert_eq!(context.get("source").unwrap(), "claude-code");
        assert_eq!(
            context.get("observedAt").unwrap(),
            "2026-09-05T07:00:00.000Z"
        );
        assert_eq!(context.get("workspaceName").unwrap(), "WS");
        assert_eq!(context.get("archived").unwrap().as_bool(), Some(false));

        // The configured model is a SELECTED identity, and it reaches the
        // model options even though no event was ever recorded.
        let selected_key = ModelIdentity {
            basis: Basis::Selected,
            provider: Some("anthropic".into()),
            name: Some("claude-opus-5".into()),
        }
        .key();
        assert_eq!(context.get("modelKey").unwrap(), selected_key.as_str());
        row_by(&out, "models", "key", &selected_key);

        // An agent with no reading keeps unknowns rather than borrowing one.
        let blank = row_by(&out, "contexts", "workspaceAgentId", &other);
        assert!(blank.get("source").unwrap().is_null());
        assert!(blank.get("observedAt").unwrap().is_null());

        // Identity filters narrow contexts; the range never does.
        let filtered = overview_of(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "workspaceAgentId": agent }),
        )
        .await;
        assert_eq!(rows(&filtered, "contexts").len(), 1);
        let by_model = overview_of(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "modelKey": selected_key }),
        )
        .await;
        assert_eq!(rows(&by_model, "contexts").len(), 1);
        assert_eq!(i64_at(&by_model, "/summary/activityCount"), 0);
    }

    // ── Backlog ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn pending_import_is_scoped_to_the_query() {
        let state = AppState::for_tests().await;
        let mine = workspace::create(&state.db, "Mine", "/tmp/mine", None)
            .await
            .unwrap()
            .id;
        let theirs = workspace::create(&state.db, "Theirs", "/tmp/theirs", None)
            .await
            .unwrap()
            .id;
        sqlx::query(
            "INSERT INTO model_usage_cursor
               (id, source_kind, source_session_id, path_fingerprint, byte_offset,
                observed_length, collector_version, workspace_id, last_verified_at)
             VALUES ('c1', 'claude-code', 's1', 'fp1', 10, 99, 'v1', ?1, '2026-09-05T00:00:00.000Z')",
        )
        .bind(&theirs)
        .execute(&state.db)
        .await
        .unwrap();

        let global = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(
            global.pointer("/coverage/pendingImport").unwrap().as_bool(),
            Some(true)
        );
        let scoped = overview_of(
            &state,
            json!({ "days": 30, "timeZone": "UTC", "workspaceId": mine }),
        )
        .await;
        assert_eq!(
            scoped.pointer("/coverage/pendingImport").unwrap().as_bool(),
            Some(false),
            "another workspace's backlog is not this scope's backlog"
        );
    }

    /// A cursor that has read everything it saw is not backlog.
    #[tokio::test]
    async fn a_caught_up_cursor_is_not_pending() {
        let state = AppState::for_tests().await;
        sqlx::query(
            "INSERT INTO model_usage_cursor
               (id, source_kind, source_session_id, path_fingerprint, byte_offset,
                observed_length, collector_version, last_verified_at)
             VALUES ('c1', 'claude-code', 's1', 'fp1', 99, 99, 'v1', '2026-09-05T00:00:00.000Z')",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        assert_eq!(
            out.pointer("/coverage/pendingImport").unwrap().as_bool(),
            Some(false)
        );
    }

    // ── Buckets over stored data ─────────────────────────────────────────

    /// A boundary event belongs to the LATER bucket (half-open), and an empty
    /// day yields a row with zero activity rather than no row at all.
    #[tokio::test]
    async fn daily_buckets_are_half_open_and_never_skipped() {
        let state = AppState::for_tests().await;
        let tz: Tz = "UTC".parse().unwrap();
        let now = Utc::now();
        let today_start = local_day_start(tz, now.date_naive()).unwrap();

        insert_event(&state.db, &ev("boundary", today_start))
            .await
            .unwrap();
        insert_event(
            &state.db,
            &ev("yesterday", today_start - Duration::minutes(1)),
        )
        .await
        .unwrap();

        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        let daily = rows(&out, "daily");
        assert_eq!(daily.len(), 30);
        let today = daily.last().unwrap();
        let yesterday = &daily[daily.len() - 2];
        assert_eq!(
            today.get("activityCount").unwrap().as_i64(),
            Some(1),
            "midnight belongs to the day it starts"
        );
        assert_eq!(yesterday.get("activityCount").unwrap().as_i64(), Some(1));
        assert_eq!(daily[0].get("activityCount").unwrap().as_i64(), Some(0));
        assert_eq!(i64_at(&out, "/summary/activityCount"), 2);
    }

    /// Events older than the range never leak into it.
    #[tokio::test]
    async fn events_outside_the_range_are_excluded() {
        let state = AppState::for_tests().await;
        insert_event(&state.db, &ev("ancient", Utc::now() - Duration::days(120)))
            .await
            .unwrap();
        let out = overview_of(&state, json!({ "days": 90, "timeZone": "UTC" })).await;
        assert_eq!(i64_at(&out, "/summary/activityCount"), 0);
    }

    /// Review ab722021: model-less events from two providers must be ONE
    /// `unknown::` row (activity 2, tokens 60), not two rows sharing a key.
    #[tokio::test]
    async fn unknown_model_rows_never_duplicate_across_providers() {
        let state = AppState::for_tests().await;
        let at = Utc::now() - Duration::hours(1);
        for (key, provider) in [("a", "anthropic"), ("b", "openai")] {
            let mut e = ev(key, at);
            e.provider = Some(provider.into());
            e.input_tokens = Some(20);
            e.output_tokens = Some(10);
            insert_event(&state.db, &e).await.unwrap();
        }
        let out = overview_of(&state, json!({ "days": 30, "timeZone": "UTC" })).await;
        let by_model = rows(&out, "byModel");
        assert_eq!(by_model.len(), 1);
        assert_eq!(by_model[0].get("key").unwrap(), "unknown::");
        assert_eq!(by_model[0].get("activityCount").unwrap().as_i64(), Some(2));
        assert_eq!(
            by_model[0].get("measuredTokens").unwrap().as_i64(),
            Some(60)
        );
        let keys: Vec<&str> = rows(&out, "models")
            .iter()
            .map(|m| m.get("key").unwrap().as_str().unwrap())
            .collect();
        assert_eq!(keys.iter().filter(|k| **k == "unknown::").count(), 1);
    }
}

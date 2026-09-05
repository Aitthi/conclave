//! Measured model-usage repository (migration 0030).
//!
//! One relation, `model_usage_event`, is the only activity source. A row is one
//! observed completed model response OR one successful non-persistent draft
//! invocation — never a user turn, a terminal chunk, or a context sample.
//!
//! # Why raw `sqlx` instead of chain-builder here
//!
//! `repo/mod.rs` prefers chain-builder for CRUD and allows raw `sqlx` "where
//! chain-builder is awkward". Everything below is either an idempotent upsert
//! (`ON CONFLICT DO NOTHING`) or a grouped aggregate over a bucket CTE; neither
//! is expressible in the builder, so this module is raw SQL throughout.
//!
//! # Truth rules encoded here
//!
//! * Missing usage is `NULL`, never `0`. `NULL` means "not observed".
//! * `input_tokens` already includes cached input; `output_tokens` already
//!   includes reasoning. The subset columns are provenance only and are never
//!   added into a total.
//! * A `conflict` row stays ONE activity but contributes no measured tokens.
//! * Aggregation never invents a bucket: the caller supplies the exact
//!   half-open UTC intervals, computed from a real IANA calendar.
//!
//! # `AssertSqlSafe` audit
//!
//! sqlx 0.9 refuses a non-literal statement unless it is explicitly asserted
//! safe. The four aggregate statements below are assembled with `format!`, so
//! each was audited against one rule: **every interpolated fragment is a
//! compile-time constant, and every runtime value is a bind parameter.**
//!
//! * `AGG_COLUMNS` is a `const &'static str`.
//! * The scope fragment from [`scope_predicate`] is built only from string
//!   literals; each literal appends a `?` placeholder and pushes the caller's
//!   value onto the bind list, so no scope value ever reaches the SQL text.
//! * `GroupColumn::sql` is an enum method returning one of two literals, which
//!   is why grouping takes an enum instead of a column name.
//! * The hidden-workspace exclusion list interpolates `"?"` repeated N times —
//!   again punctuation only; every id is pushed onto the bind list.
//! * `CURSOR_COLUMNS` is a `const &'static str` shared by the cursor SELECT and
//!   INSERT so the two can never disagree about column order; `MODEL_PROVIDER`
//!   is likewise a const expression shared by SELECT and GROUP BY.
//! * The bucket `VALUES` list is `"(?, ?, ?)"` repeated `buckets.len()` times —
//!   placeholder punctuation only; the keys and boundaries are all bound.
//!
//! A test at the bottom of this module feeds SQL metacharacters through every
//! scope field and asserts they are matched as literal data.

use serde::Serialize;
use sqlx::{AssertSqlSafe, Row, SqlitePool};

// ── Canonical timestamps ─────────────────────────────────────────────────────

/// Format every timestamp this feature stores or compares.
///
/// Aggregation compares `occurred_at` LEXICOGRAPHICALLY against bucket bounds
/// (SQLite has no date type), so lexicographic order must equal chronological
/// order. That holds only when every value shares one UTC format with a FIXED
/// fraction width: `2026-09-05T04:12:00.500Z` sorts BEFORE `2026-09-05T04:12:00Z`
/// because `'.' < 'Z'`, so mixed precision would silently mis-bucket events.
///
/// Every collector normalizes its source timestamp through this function before
/// writing `model_usage_event.occurred_at` / `recorded_at`, coverage interval
/// bounds and cursor timestamps; `commands::usage` builds its bucket bounds the
/// same way.
pub fn canonical_ts(at: chrono::DateTime<chrono::Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

// ── Scope helpers ────────────────────────────────────────────────────────────

/// Ids of HIDDEN internal workspaces (the skill-draft scratch workspaces).
///
/// Lives here rather than in `repo::workspace` because it exists only to feed
/// [`UsageScope::exclude_workspace_ids`] — usage is the one reader that needs
/// the hidden set, and `repo/workspace.rs` is outside this feature's boundary.
pub async fn hidden_workspace_ids(pool: &SqlitePool) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar("SELECT id FROM workspace WHERE hidden = 1")
        .fetch_all(pool)
        .await
}

/// The scope of one importer cursor that still has unread bytes.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PendingCursorScope {
    pub source_kind: String,
    pub workspace_id: Option<String>,
    pub workspace_agent_id: Option<String>,
}

/// Every importer cursor with backlog (`byte_offset < observed_length`).
///
/// This is the only honest, already-persisted signal that a collector has work
/// left: the cursor is written in the SAME transaction as the events it
/// produced, so it survives restart and needs no live worker to answer.
/// `usage.overview` filters these by scope and reports `coverage.pendingImport`.
pub async fn pending_cursor_scopes(pool: &SqlitePool) -> sqlx::Result<Vec<PendingCursorScope>> {
    sqlx::query_as(
        "SELECT source_kind, workspace_id, workspace_agent_id
           FROM model_usage_cursor
          WHERE byte_offset < observed_length",
    )
    .fetch_all(pool)
    .await
}

// ── Importer cursors ─────────────────────────────────────────────────────────

/// One transcript file's import position. Written in the SAME transaction as
/// the events it produced (see [`upsert_cursor`]), so a crash between the two
/// is impossible and a replay from the previous offset is a no-op through
/// `event_key`.
#[allow(dead_code)] // consumed by the transcript importer landing next in this lane
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct CursorRow {
    pub id: String,
    pub source_kind: String,
    pub source_session_id: String,
    /// Stable identity of the file itself (not its path), so rotation or
    /// replacement is detectable.
    pub path_fingerprint: String,
    pub byte_offset: i64,
    pub observed_length: i64,
    pub collector_version: String,
    pub workspace_id: Option<String>,
    pub workspace_agent_id: Option<String>,
    pub verified_owner: Option<String>,
    pub verified_cwd: Option<String>,
    /// Bounded parser continuation metadata — never raw transcript text.
    pub parser_state: Option<String>,
    pub last_verified_at: String,
}

#[allow(dead_code)] // see CursorRow
const CURSOR_COLUMNS: &str = "id, source_kind, source_session_id, path_fingerprint, byte_offset,
        observed_length, collector_version, workspace_id, workspace_agent_id,
        verified_owner, verified_cwd, parser_state, last_verified_at";

/// The cursor for one (source, file) pair, if the importer has seen it.
#[allow(dead_code)] // see CursorRow
pub async fn get_cursor<'e, E>(
    executor: E,
    source_kind: &str,
    path_fingerprint: &str,
) -> sqlx::Result<Option<CursorRow>>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query_as(AssertSqlSafe(format!(
        "SELECT {CURSOR_COLUMNS} FROM model_usage_cursor
          WHERE source_kind = ?1 AND path_fingerprint = ?2"
    )))
    .bind(source_kind)
    .bind(path_fingerprint)
    .fetch_optional(executor)
    .await
}

/// Insert or advance a cursor. The `(source_kind, path_fingerprint)` pair is
/// the identity; everything else is replaced by the caller's latest view.
#[allow(dead_code)] // see CursorRow
pub async fn upsert_cursor<'e, E>(executor: E, row: &CursorRow) -> sqlx::Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO model_usage_cursor ({CURSOR_COLUMNS})
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(source_kind, path_fingerprint) DO UPDATE SET
            source_session_id  = excluded.source_session_id,
            byte_offset        = excluded.byte_offset,
            observed_length    = excluded.observed_length,
            collector_version  = excluded.collector_version,
            workspace_id       = excluded.workspace_id,
            workspace_agent_id = excluded.workspace_agent_id,
            verified_owner     = excluded.verified_owner,
            verified_cwd       = excluded.verified_cwd,
            parser_state       = excluded.parser_state,
            last_verified_at   = excluded.last_verified_at"
    )))
    .bind(&row.id)
    .bind(&row.source_kind)
    .bind(&row.source_session_id)
    .bind(&row.path_fingerprint)
    .bind(row.byte_offset)
    .bind(row.observed_length)
    .bind(&row.collector_version)
    .bind(&row.workspace_id)
    .bind(&row.workspace_agent_id)
    .bind(&row.verified_owner)
    .bind(&row.verified_cwd)
    .bind(&row.parser_state)
    .bind(&row.last_verified_at)
    .execute(executor)
    .await?;
    Ok(())
}

// ── Coverage recording ───────────────────────────────────────────────────────

/// What a collector observed over one interval; the write-side twin of
/// [`CoverageIntervalRow`] without the storage id.
#[allow(dead_code)] // see CursorRow
#[derive(Debug, Clone, PartialEq)]
pub struct ObservedInterval<'a> {
    pub workspace_id: Option<&'a str>,
    pub workspace_agent_id: Option<&'a str>,
    pub source_kind: &'a str,
    /// `end >= start`; stored through [`canonical_ts`].
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
    pub state: &'a str,
    pub diagnostic_code: Option<&'a str>,
    pub collector_version: &'a str,
    pub last_verified_at: chrono::DateTime<chrono::Utc>,
}

/// Record an observation, merging it into an existing interval of the SAME
/// scope, source, state and diagnostic when the two touch or overlap.
///
/// A collector ticks every few seconds, so without merging the table would
/// grow by one row per tick per source forever. Merging is safe because it is
/// exact: two intervals are joined only when their union has no hole
/// (`existing.end >= new.start && existing.start <= new.end`). A gap — a
/// collector that was down — therefore always starts a NEW row, which is what
/// lets the reader see the gap. Intervals of different state (complete vs
/// partial) or different diagnostic never merge, so a partial stretch cannot
/// disappear inside a complete one.
///
/// Takes a connection rather than a pool so the caller can run it inside the
/// same transaction as the events and cursor it describes.
#[allow(dead_code)] // see CursorRow
pub async fn record_coverage(
    conn: &mut sqlx::SqliteConnection,
    observed: &ObservedInterval<'_>,
) -> sqlx::Result<()> {
    let start = canonical_ts(observed.start);
    let end = canonical_ts(observed.end);
    let last_verified_at = canonical_ts(observed.last_verified_at);
    let existing: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, interval_start, interval_end
           FROM model_usage_coverage
          WHERE source_kind = ?1
            AND workspace_id IS ?2
            AND workspace_agent_id IS ?3
            AND state = ?4
            AND diagnostic_code IS ?5
            AND interval_end >= ?6
            AND interval_start <= ?7
          ORDER BY interval_start ASC
          LIMIT 1",
    )
    .bind(observed.source_kind)
    .bind(observed.workspace_id)
    .bind(observed.workspace_agent_id)
    .bind(observed.state)
    .bind(observed.diagnostic_code)
    .bind(&start)
    .bind(&end)
    .fetch_optional(&mut *conn)
    .await?;

    match existing {
        Some((id, existing_start, existing_end)) => {
            // Canonical strings order exactly like the instants they encode.
            let merged_start = if start < existing_start {
                &start
            } else {
                &existing_start
            };
            let merged_end = if end > existing_end {
                &end
            } else {
                &existing_end
            };
            sqlx::query(
                "UPDATE model_usage_coverage
                    SET interval_start = ?2, interval_end = ?3,
                        collector_version = ?4, last_verified_at = ?5
                  WHERE id = ?1",
            )
            .bind(&id)
            .bind(merged_start)
            .bind(merged_end)
            .bind(observed.collector_version)
            .bind(&last_verified_at)
            .execute(&mut *conn)
            .await?;
        }
        None => {
            insert_coverage(
                &mut *conn,
                &CoverageIntervalRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    workspace_id: observed.workspace_id.map(str::to_owned),
                    workspace_agent_id: observed.workspace_agent_id.map(str::to_owned),
                    source_kind: observed.source_kind.to_owned(),
                    interval_start: start,
                    interval_end: end,
                    state: observed.state.to_owned(),
                    collector_version: observed.collector_version.to_owned(),
                    diagnostic_code: observed.diagnostic_code.map(str::to_owned),
                    last_verified_at,
                },
            )
            .await?;
        }
    }
    Ok(())
}

// ── Event insertion ──────────────────────────────────────────────────────────

/// One event to persist. Constructed by the collectors; every optional field is
/// `None` when the source did not prove it.
// Constructed by the collectors landing next in this lane (transcript,
// direct-provider and one-shot importers) and by the aggregation tests; the
// write side is deliberately merged before its callers so the read side could
// be reviewed against real stored data.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct NewUsageEvent {
    pub id: String,
    pub event_key: String,
    pub workspace_id: Option<String>,
    pub workspace_agent_id: Option<String>,
    pub session_id: Option<String>,
    pub generation: Option<i64>,
    pub source_kind: String,
    pub source_version: String,
    pub event_kind: String,
    pub source_session_id: Option<String>,
    pub source_request_id: Option<String>,
    pub source_response_id: Option<String>,
    /// The SOURCE's own timestamp. Typed, not a string, so the canonical
    /// storage format ([`canonical_ts`]) is applied here and cannot be skipped
    /// by a collector.
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    pub provider: Option<String>,
    pub requested_model: Option<String>,
    pub served_model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_write_input_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    /// The source's terminal stop reason, kept as reconciliation evidence for
    /// replayed Claude request groups (migration 0032). `None` for sources
    /// that report none and for an oversized value, which is dropped with
    /// [`STOP_REASON_OUT_OF_RANGE`] rather than truncated into apparent
    /// agreement.
    pub stop_reason: Option<String>,
    /// The source's own UNCACHED input counter. `input_tokens` is the
    /// cache-inclusive sum, which cannot recover this value when a cache
    /// component was missing; so it is stored as reported (migration 0032).
    pub source_uncached_input_tokens: Option<i64>,
    pub validity: String,
    pub diagnostic_code: Option<String>,
}

/// Longest `stop_reason` the store accepts, in characters — the 0032 schema
/// `CHECK`s the same bound.
pub const MAX_STOP_REASON_CHARS: usize = 128;

/// Diagnostic stamped on an event whose stop reason was dropped as unknown
/// because it exceeded [`MAX_STOP_REASON_CHARS`]. A counter rejection
/// ([`COUNTER_OUT_OF_RANGE`]) takes precedence when both happen.
pub const STOP_REASON_OUT_OF_RANGE: &str = "stop_reason_out_of_range";

#[allow(dead_code)]
/// Largest token counter this store accepts: 2^40 (about 1.1e12 tokens for
/// ONE response — far beyond any real model). Anything above it is not a
/// measurement, and a counter that size would also let SQLite's `SUM` overflow
/// or promote to REAL, i.e. error out or fabricate a float (review ab722021).
/// The bound is enforced twice: [`insert_event`] normalizes an out-of-range
/// counter to NULL (unknown) with [`COUNTER_OUT_OF_RANGE`], and the 0030 schema
/// `CHECK`s the same ceiling so no other writer can bypass it. With every
/// counter ≤ 2^40 a per-row `input + output` is ≤ 2^41 — the bound the split
/// summation in [`AGG_COLUMNS`] relies on. The ceiling alone is NOT what keeps
/// aggregation safe (4,194,304 such rows still overflow a plain `SUM`); the
/// split columns plus [`recombine`] are.
pub const MAX_TOKEN_COUNTER: i64 = 1 << 40;

/// Diagnostic stamped on an event whose counter(s) were dropped as unknown
/// because they were negative or above [`MAX_TOKEN_COUNTER`].
pub const COUNTER_OUT_OF_RANGE: &str = "counter_out_of_range";

impl NewUsageEvent {
    /// Derive the stored completeness from what was actually observed. This is
    /// the single place the rule lives so a collector cannot disagree with the
    /// aggregate about what "known" means.
    pub fn token_completeness(&self) -> &'static str {
        match (self.input_tokens, self.output_tokens) {
            (Some(_), Some(_)) => "known",
            (None, None) => "unknown",
            _ => "partial",
        }
    }

    /// Reject implausible counters as UNKNOWN rather than storing a number no
    /// aggregate can add safely. Each counter is judged on its own (a bad
    /// output leaves a good input known → `partial`). When a counter is
    /// dropped the rejection code REPLACES any diagnostic the collector set:
    /// the aggregate identifies damaged observations by this one code, so a
    /// collector's own note must not hide a rejection from it (review of
    /// 8770c7cf).
    pub(crate) fn normalized(&self) -> NewUsageEvent {
        fn bounded(value: Option<i64>, dropped: &mut bool) -> Option<i64> {
            match value {
                Some(v) if !(0..=MAX_TOKEN_COUNTER).contains(&v) => {
                    *dropped = true;
                    None
                }
                other => other,
            }
        }
        let mut dropped = false;
        let mut event = self.clone();
        event.input_tokens = bounded(self.input_tokens, &mut dropped);
        event.output_tokens = bounded(self.output_tokens, &mut dropped);
        event.cache_read_input_tokens = bounded(self.cache_read_input_tokens, &mut dropped);
        event.cache_write_input_tokens = bounded(self.cache_write_input_tokens, &mut dropped);
        event.reasoning_output_tokens = bounded(self.reasoning_output_tokens, &mut dropped);
        event.source_uncached_input_tokens =
            bounded(self.source_uncached_input_tokens, &mut dropped);
        let mut oversized_stop = false;
        if let Some(stop) = &self.stop_reason {
            if stop.chars().count() > MAX_STOP_REASON_CHARS {
                // Never truncated: two distinct oversized strings must not
                // become one agreeing value.
                event.stop_reason = None;
                oversized_stop = true;
            }
        }
        if let (Some(i), Some(o)) = (event.input_tokens, event.output_tokens) {
            if i.checked_add(o).is_none() {
                // Unreachable under the ceiling, kept so the invariant does not
                // silently depend on the constant above.
                event.input_tokens = None;
                event.output_tokens = None;
                dropped = true;
            }
        }
        if dropped {
            event.diagnostic_code = Some(COUNTER_OUT_OF_RANGE.to_owned());
        } else if oversized_stop {
            event.diagnostic_code = Some(STOP_REASON_OUT_OF_RANGE.to_owned());
        }
        event
    }
}

/// Insert one event, ignoring a replay of the same `event_key`.
///
/// Returns `true` when a new row landed. `ON CONFLICT DO NOTHING` is what makes
/// restart, duplicate source rows and a re-scanned file a no-op rather than
/// inflation — the caller does not need to pre-check existence.
#[allow(dead_code)] // see NewUsageEvent
pub async fn insert_event<'e, E>(executor: E, event: &NewUsageEvent) -> sqlx::Result<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let event = &event.normalized();
    let result = sqlx::query(
        "INSERT INTO model_usage_event (
            id, event_key, workspace_id, workspace_agent_id, session_id, generation,
            source_kind, source_version, event_kind,
            source_session_id, source_request_id, source_response_id,
            occurred_at, recorded_at, provider, requested_model, served_model,
            input_tokens, output_tokens,
            cache_read_input_tokens, cache_write_input_tokens, reasoning_output_tokens,
            token_completeness, validity, diagnostic_code,
            stop_reason, source_uncached_input_tokens
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
            ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27
         )
         ON CONFLICT(event_key) DO NOTHING",
    )
    .bind(&event.id)
    .bind(&event.event_key)
    .bind(&event.workspace_id)
    .bind(&event.workspace_agent_id)
    .bind(&event.session_id)
    .bind(event.generation)
    .bind(&event.source_kind)
    .bind(&event.source_version)
    .bind(&event.event_kind)
    .bind(&event.source_session_id)
    .bind(&event.source_request_id)
    .bind(&event.source_response_id)
    .bind(canonical_ts(event.occurred_at))
    .bind(canonical_ts(event.recorded_at))
    .bind(&event.provider)
    .bind(&event.requested_model)
    .bind(&event.served_model)
    .bind(event.input_tokens)
    .bind(event.output_tokens)
    .bind(event.cache_read_input_tokens)
    .bind(event.cache_write_input_tokens)
    .bind(event.reasoning_output_tokens)
    .bind(event.token_completeness())
    .bind(&event.validity)
    .bind(&event.diagnostic_code)
    .bind(&event.stop_reason)
    .bind(event.source_uncached_input_tokens)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// The stored identity of an event, for reconciling a replayed source row
/// against what was recorded first: response id, served model, stop reason,
/// the source's uncached input and every stored input-side counter. A legacy
/// row (pre-0032) carries `None` evidence, which can never AGREE with a row
/// that has it — replay against incomplete evidence stays conflicting.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StoredIdentity {
    pub source_response_id: Option<String>,
    pub served_model: Option<String>,
    pub stop_reason: Option<String>,
    pub source_uncached_input_tokens: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_write_input_tokens: Option<i64>,
    /// The stored rejection, if any. Part of the identity: a replayed block
    /// whose counters were REJECTED normalizes to the same `None` as a block
    /// that never carried them, and only the diagnostic tells the two apart
    /// (review 229a4753 C7).
    pub diagnostic_code: Option<String>,
    pub validity: String,
}

/// Read back what was recorded under `event_key`, if anything.
pub async fn stored_identity<'e, E>(
    executor: E,
    event_key: &str,
) -> sqlx::Result<Option<StoredIdentity>>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query_as(
        "SELECT source_response_id, served_model, stop_reason,
                source_uncached_input_tokens, input_tokens, output_tokens,
                cache_read_input_tokens, cache_write_input_tokens,
                diagnostic_code, validity
           FROM model_usage_event WHERE event_key = ?1",
    )
    .bind(event_key)
    .fetch_optional(executor)
    .await
}

/// Move an event's `occurred_at` FORWARD to `at` when a later agreeing source
/// row proves the response completed later than first recorded (a Claude
/// request group completes at its last block). Never moves it backwards.
pub async fn advance_occurred_at<'e, E>(
    executor: E,
    event_key: &str,
    at: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let at = canonical_ts(at);
    sqlx::query(
        "UPDATE model_usage_event SET occurred_at = ?2
          WHERE event_key = ?1 AND occurred_at < ?2",
    )
    .bind(event_key)
    .bind(at)
    .execute(executor)
    .await?;
    Ok(())
}

/// Mark an already-stored event as conflicting. It remains one activity; only
/// its measured token contribution is withdrawn (contract: a conflict "never
/// creates a second activity").
#[allow(dead_code)] // see NewUsageEvent
pub async fn mark_conflict<'e, E>(
    executor: E,
    event_key: &str,
    diagnostic_code: &str,
) -> sqlx::Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "UPDATE model_usage_event
            SET validity = 'conflict', diagnostic_code = ?2
          WHERE event_key = ?1",
    )
    .bind(event_key)
    .bind(diagnostic_code)
    .execute(executor)
    .await?;
    Ok(())
}

// ── Coverage ─────────────────────────────────────────────────────────────────

/// One observed interval for a (scope, source) pair. The ABSENCE of intervals
/// is what `none` coverage means — a gap is never stored as a row.
///
/// # Scope columns are a WIDTH, not a bucket
///
/// `NULL` in `workspace_id` / `workspace_agent_id` means UNRESTRICTED on that
/// dimension ("this collector observed everything"), NOT "the unscoped
/// events". A row therefore PROVES a query's scope only when it is at least as
/// wide as that scope; a narrower row is compatible evidence that something was
/// observed, which is exactly `partial`. `commands::usage` owns that comparison.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CoverageIntervalRow {
    pub id: String,
    pub workspace_id: Option<String>,
    pub workspace_agent_id: Option<String>,
    pub source_kind: String,
    pub interval_start: String,
    pub interval_end: String,
    pub state: String,
    pub collector_version: String,
    /// Why the interval is only partial, when the collector knows. The reserved
    /// value [`UNSUPPORTED_SOURCE`] is what lets `usage.overview` DERIVE
    /// `coverage.unsupportedSources` instead of hardcoding an empty list.
    pub diagnostic_code: Option<String>,
    pub last_verified_at: String,
}

/// Reserved [`CoverageIntervalRow::diagnostic_code`]: the collector looked at
/// this source and could not import its shape. The interval is real evidence
/// (we know we looked) but never complete.
pub const UNSUPPORTED_SOURCE: &str = "unsupported_source";

/// Record an observed interval.
#[allow(dead_code)] // see NewUsageEvent
pub async fn insert_coverage<'e, E>(executor: E, row: &CoverageIntervalRow) -> sqlx::Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO model_usage_coverage (
            id, workspace_id, workspace_agent_id, source_kind,
            interval_start, interval_end, state, collector_version,
            diagnostic_code, last_verified_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(&row.id)
    .bind(&row.workspace_id)
    .bind(&row.workspace_agent_id)
    .bind(&row.source_kind)
    .bind(&row.interval_start)
    .bind(&row.interval_end)
    .bind(&row.state)
    .bind(&row.collector_version)
    .bind(&row.diagnostic_code)
    .bind(&row.last_verified_at)
    .execute(executor)
    .await?;
    Ok(())
}

/// Every coverage interval overlapping `[start, end)`, oldest first.
///
/// Overlap (not containment) is deliberate: an interval that starts before the
/// range still proves observation inside it.
pub async fn coverage_overlapping(
    pool: &SqlitePool,
    start_utc: &str,
    end_utc: &str,
) -> sqlx::Result<Vec<CoverageIntervalRow>> {
    sqlx::query_as(
        "SELECT id, workspace_id, workspace_agent_id, source_kind,
                interval_start, interval_end, state, collector_version,
                diagnostic_code, last_verified_at
           FROM model_usage_coverage
          WHERE interval_start < ?2 AND interval_end > ?1
          ORDER BY interval_start ASC, id ASC",
    )
    .bind(start_utc)
    .bind(end_utc)
    .fetch_all(pool)
    .await
}

// ── Aggregation ──────────────────────────────────────────────────────────────

/// Scope filters for a query. `None` means "no filter"; the reserved wire ids
/// are resolved to the explicit `*_is_null` flags by the command layer so the
/// repository never has to know about `__unscoped__` / `__unassigned__`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageScope {
    pub workspace_id: Option<String>,
    /// Restrict to events with no workspace at all (the wire `__unscoped__`).
    pub workspace_unscoped: bool,
    pub workspace_agent_id: Option<String>,
    /// Restrict to events with no workspace agent (the wire `__unassigned__`).
    pub agent_unassigned: bool,
    /// Opaque model key: (provider, name, basis). `basis` decides which column
    /// the name is matched against, so a Selected row can never be silently
    /// served by a Reported one.
    pub model: Option<ModelKeyFilter>,
    /// Workspaces whose events are outside the normal aggregate — in practice
    /// the HIDDEN scratch workspaces backing skill-draft sessions, which the
    /// contract keeps out of ordinary usage scope. Unscoped (NULL-workspace)
    /// events are never excluded by this list; they are legitimate Library
    /// draft activity.
    pub exclude_workspace_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelKeyFilter {
    pub provider: Option<String>,
    pub name: Option<String>,
    pub basis: String,
}

/// One aggregated group. `bucket` is whatever the caller grouped by.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageAggregate {
    pub bucket: String,
    pub activity_count: i64,
    pub response_count: i64,
    pub invocation_count: i64,
    pub measured_event_count: i64,
    pub unknown_usage_count: i64,
    /// Sum over rows where BOTH components are known and the row is valid.
    /// `None` when no row contributed OR when the exact sum does not fit an
    /// i64 (then the matching `*_overflow` flag is set).
    pub measured_tokens: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    /// The exact sum exceeded i64: the tokens are unavailable, never rounded.
    pub measured_overflow: bool,
    pub input_overflow: bool,
    pub output_overflow: bool,
    /// Rows whose counters were rejected at insertion ([`COUNTER_OUT_OF_RANGE`]).
    /// Evidence that the observation of this group was damaged, so the command
    /// layer caps its coverage at partial (ruling on challenge 34201f49).
    pub rejected_counter_count: i64,
    /// Rows in `validity = 'conflict'`: one activity each, no tokens, and the
    /// same partial-coverage consequence.
    pub conflict_count: i64,
}

/// Build the shared `WHERE` fragment and its binds for a scope.
fn scope_predicate(scope: &UsageScope) -> (String, Vec<Option<String>>) {
    let mut sql = String::new();
    let mut binds: Vec<Option<String>> = Vec::new();
    if scope.workspace_unscoped {
        sql.push_str(" AND e.workspace_id IS NULL");
    } else if let Some(ws) = &scope.workspace_id {
        sql.push_str(" AND e.workspace_id = ?");
        binds.push(Some(ws.clone()));
    }
    if scope.agent_unassigned {
        sql.push_str(" AND e.workspace_agent_id IS NULL");
    } else if let Some(agent) = &scope.workspace_agent_id {
        sql.push_str(" AND e.workspace_agent_id = ?");
        binds.push(Some(agent.clone()));
    }
    if !scope.exclude_workspace_ids.is_empty() {
        // Placeholder punctuation only — every id is bound (see the module's
        // `AssertSqlSafe` audit). `IS NULL OR NOT IN` keeps unscoped events in.
        let holes = vec!["?"; scope.exclude_workspace_ids.len()].join(", ");
        sql.push_str(&format!(
            " AND (e.workspace_id IS NULL OR e.workspace_id NOT IN ({holes}))"
        ));
        for id in &scope.exclude_workspace_ids {
            binds.push(Some(id.clone()));
        }
    }
    if let Some(model) = &scope.model {
        match model.basis.as_str() {
            // A Reported row is one whose served model the source actually
            // observed; a Selected row is one where it did not.
            "reported" => {
                sql.push_str(" AND e.served_model IS NOT NULL AND e.served_model IS ?");
                binds.push(model.name.clone());
            }
            "selected" => {
                sql.push_str(" AND e.served_model IS NULL AND e.requested_model IS NOT NULL AND e.requested_model IS ?");
                binds.push(model.name.clone());
            }
            _ => {
                sql.push_str(" AND e.served_model IS NULL AND e.requested_model IS NULL");
            }
        }
        if model.basis != "unknown" {
            sql.push_str(" AND e.provider IS ?");
            binds.push(model.provider.clone());
        }
    }
    (sql, binds)
}

/// The aggregate projection, shared by every grouping.
///
/// `validity = 'valid'` gates only the TOKEN sums — a conflicting row is still
/// counted as activity, because the response demonstrably happened.
///
/// # Why every token sum is split into two columns
///
/// SQLite's `SUM` over integers raises "integer overflow" the moment the running
/// total passes i64::MAX, and the alternative `TOTAL` silently rounds in a
/// double. Neither is acceptable: the first turns a large history into an
/// Overview error, the second fabricates digits (review 34201f49 reproduced the
/// error with 4,194,304 rows at the [`MAX_TOKEN_COUNTER`] ceiling). So each
/// per-row value `x` (≤ 2^41 under the schema CHECK) is summed as two exact
/// integer parts, `x / 2^20` and `x % 2^20`; each part is ≤ 2^21 per row, so
/// the SQL-side sums cannot overflow before 2^42 rows — beyond any SQLite
/// table. [`recombine`] then rebuilds `hi * 2^20 + lo` in Rust with checked
/// arithmetic, and an unrepresentable total becomes `None` plus an overflow
/// flag instead of an error or a rounded number.
const AGG_COLUMNS: &str = "
    COUNT(*) AS activity_count,
    COALESCE(SUM(CASE WHEN e.event_kind = 'response'   THEN 1 ELSE 0 END), 0) AS response_count,
    COALESCE(SUM(CASE WHEN e.event_kind = 'invocation' THEN 1 ELSE 0 END), 0) AS invocation_count,
    COALESCE(SUM(CASE WHEN e.validity = 'valid' AND e.token_completeness = 'known' THEN 1 ELSE 0 END), 0) AS measured_event_count,
    COALESCE(SUM(CASE WHEN e.validity <> 'valid' OR e.token_completeness <> 'known' THEN 1 ELSE 0 END), 0) AS unknown_usage_count,
    COALESCE(SUM(CASE WHEN e.diagnostic_code = 'counter_out_of_range' THEN 1 ELSE 0 END), 0) AS rejected_counter_count,
    COALESCE(SUM(CASE WHEN e.validity = 'conflict' THEN 1 ELSE 0 END), 0) AS conflict_count,
    SUM(CASE WHEN e.validity = 'valid' AND e.token_completeness = 'known'
             THEN (e.input_tokens + e.output_tokens) / 1048576 END) AS measured_hi,
    SUM(CASE WHEN e.validity = 'valid' AND e.token_completeness = 'known'
             THEN (e.input_tokens + e.output_tokens) % 1048576 END) AS measured_lo,
    SUM(CASE WHEN e.validity = 'valid' THEN e.input_tokens  / 1048576 END) AS input_hi,
    SUM(CASE WHEN e.validity = 'valid' THEN e.input_tokens  % 1048576 END) AS input_lo,
    SUM(CASE WHEN e.validity = 'valid' THEN e.output_tokens / 1048576 END) AS output_hi,
    SUM(CASE WHEN e.validity = 'valid' THEN e.output_tokens % 1048576 END) AS output_lo
";

/// The split factor used by [`AGG_COLUMNS`]: 2^20.
const SPLIT: i64 = 1_048_576;

/// Rebuild an exact sum from its `hi`/`lo` parts.
///
/// Returns `(None, false)` when no row contributed (both parts NULL),
/// `(Some(total), false)` when the total fits, and `(None, true)` when the true
/// total does not fit an i64 — unavailable, not rounded, not zero.
pub fn recombine(hi: Option<i64>, lo: Option<i64>) -> (Option<i64>, bool) {
    match (hi, lo) {
        (Some(hi), Some(lo)) => match hi.checked_mul(SPLIT).and_then(|h| h.checked_add(lo)) {
            Some(total) => (Some(total), false),
            None => (None, true),
        },
        _ => (None, false),
    }
}

fn read_aggregate(row: &sqlx::sqlite::SqliteRow, bucket: String) -> UsageAggregate {
    let (measured_tokens, measured_overflow) =
        recombine(row.get("measured_hi"), row.get("measured_lo"));
    let (input_tokens, input_overflow) = recombine(row.get("input_hi"), row.get("input_lo"));
    let (output_tokens, output_overflow) = recombine(row.get("output_hi"), row.get("output_lo"));
    UsageAggregate {
        bucket,
        activity_count: row.get("activity_count"),
        response_count: row.get("response_count"),
        invocation_count: row.get("invocation_count"),
        measured_event_count: row.get("measured_event_count"),
        unknown_usage_count: row.get("unknown_usage_count"),
        measured_tokens,
        input_tokens,
        output_tokens,
        measured_overflow,
        input_overflow,
        output_overflow,
        rejected_counter_count: row.get("rejected_counter_count"),
        conflict_count: row.get("conflict_count"),
    }
}

/// Aggregate every event in `[start_utc, end_utc)` into one total.
pub async fn aggregate_range(
    pool: &SqlitePool,
    scope: &UsageScope,
    start_utc: &str,
    end_utc: &str,
) -> sqlx::Result<UsageAggregate> {
    let (pred, binds) = scope_predicate(scope);
    let sql = format!(
        "SELECT {AGG_COLUMNS}
           FROM model_usage_event e
          WHERE e.occurred_at >= ? AND e.occurred_at < ?{pred}"
    );
    let mut q = sqlx::query(AssertSqlSafe(sql))
        .bind(start_utc)
        .bind(end_utc);
    for b in &binds {
        q = q.bind(b.clone());
    }
    let row = q.fetch_one(pool).await?;
    Ok(read_aggregate(&row, String::new()))
}

/// Aggregate per caller-supplied half-open UTC bucket.
///
/// The buckets arrive as a `VALUES` CTE rather than 90 separate queries, and
/// they are computed from a real IANA calendar by the command layer — SQLite
/// has no timezone database, so bucket boundaries are never derived in SQL.
pub async fn aggregate_buckets(
    pool: &SqlitePool,
    scope: &UsageScope,
    buckets: &[(String, String, String)],
) -> sqlx::Result<Vec<UsageAggregate>> {
    if buckets.is_empty() {
        return Ok(Vec::new());
    }
    let (pred, binds) = scope_predicate(scope);
    let values = buckets
        .iter()
        .map(|_| "(?, ?, ?)")
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "WITH bucket(key, start_utc, end_utc) AS (VALUES {values})
         SELECT b.key AS bucket, {AGG_COLUMNS}
           FROM bucket b
           JOIN model_usage_event e
             ON e.occurred_at >= b.start_utc AND e.occurred_at < b.end_utc
          WHERE 1 = 1{pred}
          GROUP BY b.key"
    );
    let mut q = sqlx::query(AssertSqlSafe(sql));
    for (key, start, end) in buckets {
        q = q.bind(key).bind(start).bind(end);
    }
    for b in &binds {
        q = q.bind(b.clone());
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|r| {
            let bucket: String = r.get("bucket");
            read_aggregate(r, bucket)
        })
        .collect())
}

/// Group by an event column (`workspace_id`, `workspace_agent_id`) over the
/// whole range. A NULL group key comes back as `None`, which the command layer
/// maps to the reserved `__unscoped__` / `__unassigned__` wire bucket.
pub async fn aggregate_by_column(
    pool: &SqlitePool,
    scope: &UsageScope,
    column: GroupColumn,
    start_utc: &str,
    end_utc: &str,
) -> sqlx::Result<Vec<(Option<String>, UsageAggregate)>> {
    let (pred, binds) = scope_predicate(scope);
    let col = column.sql();
    let sql = format!(
        "SELECT {col} AS group_key, {AGG_COLUMNS}
           FROM model_usage_event e
          WHERE e.occurred_at >= ? AND e.occurred_at < ?{pred}
          GROUP BY {col}"
    );
    let mut q = sqlx::query(AssertSqlSafe(sql))
        .bind(start_utc)
        .bind(end_utc);
    for b in &binds {
        q = q.bind(b.clone());
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|r| {
            let key: Option<String> = r.get("group_key");
            let agg = read_aggregate(r, key.clone().unwrap_or_default());
            (key, agg)
        })
        .collect())
}

/// Columns this repository is willing to group by. An enum rather than a
/// `&str` so no caller can interpolate arbitrary SQL into the statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupColumn {
    Workspace,
    WorkspaceAgent,
}

impl GroupColumn {
    fn sql(self) -> &'static str {
        match self {
            GroupColumn::Workspace => "e.workspace_id",
            GroupColumn::WorkspaceAgent => "e.workspace_agent_id",
        }
    }
}

/// The provider column of a model identity: NULL whenever the identity itself
/// is unknown, so every model-less event shares one group.
const MODEL_PROVIDER: &str =
    "CASE WHEN e.served_model IS NULL AND e.requested_model IS NULL THEN NULL ELSE e.provider END";

/// One model identity observed in the range, with its aggregate.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelAggregate {
    pub provider: Option<String>,
    pub served_model: Option<String>,
    pub requested_model: Option<String>,
    pub aggregate: UsageAggregate,
}

/// Group by model identity. Reported and Selected form SEPARATE groups even for
/// the same provider and name, so a requested model can never be presented as a
/// served one. An event with NO model at all belongs to the single `unknown`
/// identity regardless of provider — the wire key for unknown carries no
/// provider, so grouping by it here would emit duplicate `unknown::` rows
/// (review ab722021).
pub async fn aggregate_by_model(
    pool: &SqlitePool,
    scope: &UsageScope,
    start_utc: &str,
    end_utc: &str,
) -> sqlx::Result<Vec<ModelAggregate>> {
    let (pred, binds) = scope_predicate(scope);
    let sql = format!(
        "SELECT {MODEL_PROVIDER} AS provider,
                e.served_model AS served_model,
                CASE WHEN e.served_model IS NULL THEN e.requested_model END AS requested_model,
                {AGG_COLUMNS}
           FROM model_usage_event e
          WHERE e.occurred_at >= ? AND e.occurred_at < ?{pred}
          GROUP BY {MODEL_PROVIDER}, e.served_model,
                   CASE WHEN e.served_model IS NULL THEN e.requested_model END"
    );
    let mut q = sqlx::query(AssertSqlSafe(sql))
        .bind(start_utc)
        .bind(end_utc);
    for b in &binds {
        q = q.bind(b.clone());
    }
    let rows = q.fetch_all(pool).await?;
    Ok(rows
        .iter()
        .map(|r| ModelAggregate {
            provider: r.get("provider"),
            served_model: r.get("served_model"),
            requested_model: r.get("requested_model"),
            aggregate: read_aggregate(r, String::new()),
        })
        .collect())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    fn at(text: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(text)
            .expect("test timestamp")
            .with_timezone(&chrono::Utc)
    }

    /// Minimal valid event; each test overrides only what it is about.
    fn event(key: &str, occurred_at: &str) -> NewUsageEvent {
        NewUsageEvent {
            id: format!("id-{key}"),
            event_key: key.to_owned(),
            workspace_id: Some("ws".into()),
            workspace_agent_id: Some("wa".into()),
            session_id: None,
            generation: None,
            source_kind: "claude_transcript".into(),
            source_version: "v1".into(),
            event_kind: "response".into(),
            source_session_id: None,
            source_request_id: None,
            source_response_id: None,
            occurred_at: at(occurred_at),
            recorded_at: at("2026-09-05T00:00:00.000Z"),
            provider: Some("anthropic".into()),
            requested_model: None,
            served_model: Some("claude-fable-5-1".into()),
            input_tokens: Some(100),
            output_tokens: Some(20),
            cache_read_input_tokens: Some(90),
            cache_write_input_tokens: None,
            reasoning_output_tokens: None,
            stop_reason: None,
            source_uncached_input_tokens: None,
            validity: "valid".into(),
            diagnostic_code: None,
        }
    }

    const RANGE_START: &str = "2026-09-01T00:00:00.000Z";
    const RANGE_END: &str = "2026-09-30T00:00:00.000Z";

    async fn agg(pool: &SqlitePool, scope: &UsageScope) -> UsageAggregate {
        aggregate_range(pool, scope, RANGE_START, RANGE_END)
            .await
            .unwrap()
    }

    /// Replay is the whole reason `event_key` is UNIQUE: re-importing the same
    /// source record must not add a second activity or double the tokens.
    #[tokio::test]
    async fn replaying_the_same_event_key_cannot_double_count() {
        let pool = connect_in_memory().await;
        let e = event("claude-code:v1:s:r", "2026-09-10T12:00:00.000Z");
        assert!(insert_event(&pool, &e).await.unwrap(), "first insert lands");
        assert!(
            !insert_event(&pool, &e).await.unwrap(),
            "replay must be a no-op"
        );
        // A different local id for the same source identity is still one event.
        let mut same_source = e.clone();
        same_source.id = "another-uuid".into();
        assert!(!insert_event(&pool, &same_source).await.unwrap());

        let total = agg(&pool, &UsageScope::default()).await;
        assert_eq!(total.activity_count, 1);
        assert_eq!(total.measured_tokens, Some(120));
    }

    /// D4: measured tokens are NULL, not zero, whenever nothing was measured —
    /// and a row whose usage was never observed counts as activity but not as
    /// a measurement.
    #[tokio::test]
    async fn unknown_usage_counts_as_activity_but_never_as_zero_tokens() {
        let pool = connect_in_memory().await;
        let mut unknown = event("k1", "2026-09-10T12:00:00.000Z");
        unknown.input_tokens = None;
        unknown.output_tokens = None;
        assert_eq!(unknown.token_completeness(), "unknown");
        insert_event(&pool, &unknown).await.unwrap();

        let total = agg(&pool, &UsageScope::default()).await;
        assert_eq!(total.activity_count, 1, "the response still happened");
        assert_eq!(total.measured_event_count, 0);
        assert_eq!(total.unknown_usage_count, 1);
        assert_eq!(
            total.measured_tokens, None,
            "unobserved usage is unknown, never a zero total"
        );
    }

    /// A source that reports a genuine zero is NOT the same as an unobserved
    /// one: it is a measurement whose value happens to be 0.
    #[tokio::test]
    async fn a_reported_zero_is_measured_not_unknown() {
        let pool = connect_in_memory().await;
        let mut zero = event("k1", "2026-09-10T12:00:00.000Z");
        zero.input_tokens = Some(0);
        zero.output_tokens = Some(0);
        insert_event(&pool, &zero).await.unwrap();

        let total = agg(&pool, &UsageScope::default()).await;
        assert_eq!(total.measured_event_count, 1);
        assert_eq!(total.unknown_usage_count, 0);
        assert_eq!(total.measured_tokens, Some(0));
    }

    /// A conflicting group stays one activity and withdraws only its tokens.
    #[tokio::test]
    async fn conflict_keeps_the_activity_and_drops_only_the_measurement() {
        let pool = connect_in_memory().await;
        insert_event(&pool, &event("k1", "2026-09-10T12:00:00.000Z"))
            .await
            .unwrap();
        mark_conflict(&pool, "k1", "claude_group_disagreement")
            .await
            .unwrap();

        let total = agg(&pool, &UsageScope::default()).await;
        assert_eq!(total.activity_count, 1, "never a second activity");
        assert_eq!(total.response_count, 1);
        assert_eq!(total.measured_event_count, 0);
        assert_eq!(total.unknown_usage_count, 1);
        assert_eq!(total.measured_tokens, None);
    }

    /// D1: an unscoped draft is real activity that belongs to no workspace, and
    /// a real workspace filter must not sweep it up.
    #[tokio::test]
    async fn unscoped_events_are_separable_from_workspace_scoped_ones() {
        let pool = connect_in_memory().await;
        let mut scoped = event("scoped", "2026-09-10T12:00:00.000Z");
        scoped.event_kind = "invocation".into();
        insert_event(&pool, &scoped).await.unwrap();

        let mut unscoped = event("unscoped", "2026-09-10T13:00:00.000Z");
        unscoped.event_kind = "invocation".into();
        unscoped.workspace_id = None;
        unscoped.workspace_agent_id = None;
        unscoped.source_kind = "draft_oneshot".into();
        insert_event(&pool, &unscoped).await.unwrap();

        let all = agg(&pool, &UsageScope::default()).await;
        assert_eq!(all.activity_count, 2, "default scope includes both");
        assert_eq!(all.invocation_count, 2);

        let only_ws = agg(
            &pool,
            &UsageScope {
                workspace_id: Some("ws".into()),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            only_ws.activity_count, 1,
            "a real workspace filter excludes unscoped events"
        );

        let only_unscoped = agg(
            &pool,
            &UsageScope {
                workspace_unscoped: true,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(only_unscoped.activity_count, 1);

        let unassigned = agg(
            &pool,
            &UsageScope {
                agent_unassigned: true,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            unassigned.activity_count, 1,
            "__unassigned__ means workspace_agent_id IS NULL"
        );

        // Every grouping must reconcile to the same total.
        let by_ws = aggregate_by_column(
            &pool,
            &UsageScope::default(),
            GroupColumn::Workspace,
            RANGE_START,
            RANGE_END,
        )
        .await
        .unwrap();
        assert_eq!(
            by_ws.len(),
            2,
            "one real workspace group plus the NULL group"
        );
        assert!(by_ws.iter().any(|(k, _)| k.is_none()), "NULL group present");
        assert_eq!(
            by_ws.iter().map(|(_, a)| a.activity_count).sum::<i64>(),
            all.activity_count
        );
    }

    /// Reported and Selected are separate model identities even for the same
    /// provider and name — a selected model must never be served as a reported
    /// one.
    #[tokio::test]
    async fn reported_and_selected_models_are_distinct_identities() {
        let pool = connect_in_memory().await;
        let mut reported = event("reported", "2026-09-10T12:00:00.000Z");
        reported.served_model = Some("gpt-6-astra".into());
        reported.requested_model = Some("gpt-6-astra".into());
        reported.provider = Some("openai".into());
        insert_event(&pool, &reported).await.unwrap();

        let mut selected = event("selected", "2026-09-10T13:00:00.000Z");
        selected.served_model = None;
        selected.requested_model = Some("gpt-6-astra".into());
        selected.provider = Some("openai".into());
        insert_event(&pool, &selected).await.unwrap();

        let rows = aggregate_by_model(&pool, &UsageScope::default(), RANGE_START, RANGE_END)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "same name, two bases, two rows");

        let only_selected = agg(
            &pool,
            &UsageScope {
                model: Some(ModelKeyFilter {
                    provider: Some("openai".into()),
                    name: Some("gpt-6-astra".into()),
                    basis: "selected".into(),
                }),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            only_selected.activity_count, 1,
            "the selected filter must not pick up the reported row"
        );
    }

    /// Buckets are half-open: an event exactly on a boundary belongs to the
    /// later bucket, and one outside the range is not counted at all.
    #[tokio::test]
    async fn buckets_are_half_open_and_never_fabricate_events() {
        let pool = connect_in_memory().await;
        insert_event(&pool, &event("before", "2026-09-09T23:59:59.000Z"))
            .await
            .unwrap();
        insert_event(&pool, &event("boundary", "2026-09-10T00:00:00.000Z"))
            .await
            .unwrap();
        insert_event(&pool, &event("inside", "2026-09-10T12:00:00.000Z"))
            .await
            .unwrap();

        let buckets = vec![
            (
                "2026-09-09".to_string(),
                "2026-09-09T00:00:00.000Z".to_string(),
                "2026-09-10T00:00:00.000Z".to_string(),
            ),
            (
                "2026-09-10".to_string(),
                "2026-09-10T00:00:00.000Z".to_string(),
                "2026-09-11T00:00:00.000Z".to_string(),
            ),
        ];
        let rows = aggregate_buckets(&pool, &UsageScope::default(), &buckets)
            .await
            .unwrap();
        let day = |key: &str| {
            rows.iter()
                .find(|r| r.bucket == key)
                .map(|r| r.activity_count)
                .unwrap_or(0)
        };
        assert_eq!(day("2026-09-09"), 1, "boundary belongs to the later bucket");
        assert_eq!(day("2026-09-10"), 2);
        // An empty bucket produces NO row — the caller decides whether that is
        // a measured zero or unknown, using coverage.
        let empty = aggregate_buckets(
            &pool,
            &UsageScope::default(),
            &[(
                "2026-09-20".to_string(),
                "2026-09-20T00:00:00.000Z".to_string(),
                "2026-09-21T00:00:00.000Z".to_string(),
            )],
        )
        .await
        .unwrap();
        assert!(empty.is_empty(), "no events means no row, not a zero row");
    }

    /// The `AssertSqlSafe` audit in this module's header claims every scope
    /// value is a bind. This proves it: SQL metacharacters are matched as
    /// literal data and cannot alter the statement.
    #[tokio::test]
    async fn scope_values_are_bound_not_interpolated() {
        let pool = connect_in_memory().await;
        let injection = "' OR 1=1 --";
        let mut evil = event("evil", "2026-09-10T12:00:00.000Z");
        evil.workspace_id = Some(injection.to_owned());
        insert_event(&pool, &evil).await.unwrap();
        insert_event(&pool, &event("normal", "2026-09-10T12:00:00.000Z"))
            .await
            .unwrap();

        let matched = agg(
            &pool,
            &UsageScope {
                workspace_id: Some(injection.to_owned()),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            matched.activity_count, 1,
            "the metacharacter string matched exactly one row as literal data"
        );

        let no_match = agg(
            &pool,
            &UsageScope {
                workspace_id: Some("' OR 1=1".into()),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(no_match.activity_count, 0, "no injection, no escape hatch");
    }

    /// Coverage overlap, not containment: an interval that starts before the
    /// window still proves observation inside it.
    #[tokio::test]
    async fn coverage_returns_intervals_overlapping_the_window() {
        let pool = connect_in_memory().await;
        let row = |id: &str, start: &str, end: &str| CoverageIntervalRow {
            id: id.into(),
            workspace_id: Some("ws".into()),
            workspace_agent_id: None,
            source_kind: "claude_transcript".into(),
            interval_start: start.into(),
            interval_end: end.into(),
            state: "complete".into(),
            collector_version: "v1".into(),
            diagnostic_code: None,
            last_verified_at: "2026-09-30T00:00:00.000Z".into(),
        };
        insert_coverage(
            &pool,
            &row(
                "straddles",
                "2026-08-20T00:00:00.000Z",
                "2026-09-05T00:00:00.000Z",
            ),
        )
        .await
        .unwrap();
        insert_coverage(
            &pool,
            &row(
                "outside",
                "2026-07-01T00:00:00.000Z",
                "2026-07-02T00:00:00.000Z",
            ),
        )
        .await
        .unwrap();

        let found = coverage_overlapping(&pool, RANGE_START, RANGE_END)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "straddles");
    }

    // ── Cursors and coverage recording ───────────────────────────────────

    fn cursor(offset: i64, length: i64) -> CursorRow {
        CursorRow {
            id: "cursor-1".into(),
            source_kind: "claude-code".into(),
            source_session_id: "sess-1".into(),
            path_fingerprint: "fp-1".into(),
            byte_offset: offset,
            observed_length: length,
            collector_version: "v1".into(),
            workspace_id: Some("ws".into()),
            workspace_agent_id: Some("wa".into()),
            verified_owner: Some("wa".into()),
            verified_cwd: Some("/tmp/ws".into()),
            parser_state: None,
            last_verified_at: "2026-09-05T00:00:00.000Z".into(),
        }
    }

    #[tokio::test]
    async fn a_cursor_advances_in_place_and_stays_one_row() {
        let pool = connect_in_memory().await;
        upsert_cursor(&pool, &cursor(0, 100)).await.unwrap();
        let mut advanced = cursor(100, 250);
        advanced.id = "a-different-id-is-ignored".into();
        advanced.parser_state = Some(r#"{"pending":"req-9"}"#.into());
        upsert_cursor(&pool, &advanced).await.unwrap();

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_usage_cursor")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 1, "same file, one cursor");
        let got = get_cursor(&pool, "claude-code", "fp-1")
            .await
            .unwrap()
            .expect("cursor exists");
        assert_eq!(got.byte_offset, 100);
        assert_eq!(got.observed_length, 250);
        assert_eq!(got.parser_state.as_deref(), Some(r#"{"pending":"req-9"}"#));
        assert_eq!(
            got.id, "cursor-1",
            "identity is the (source, fingerprint) pair"
        );
        assert!(get_cursor(&pool, "codex", "fp-1").await.unwrap().is_none());
    }

    fn observed<'a>(start: &str, end: &str, state: &'a str) -> ObservedInterval<'a> {
        ObservedInterval {
            workspace_id: Some("ws"),
            workspace_agent_id: None,
            source_kind: "claude-code",
            start: at(start),
            end: at(end),
            state,
            diagnostic_code: None,
            collector_version: "v1",
            last_verified_at: at(end),
        }
    }

    /// `connect_in_memory` is a ONE-connection pool: hold the connection only
    /// for the write, or a following pool read deadlocks.
    async fn record(pool: &SqlitePool, observed: &ObservedInterval<'_>) {
        let mut conn = pool.acquire().await.unwrap();
        record_coverage(&mut conn, observed).await.unwrap();
    }

    async fn coverage_rows(pool: &SqlitePool) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "SELECT interval_start, interval_end, state FROM model_usage_coverage
              ORDER BY interval_start ASC",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// Ticks that touch or overlap collapse into one interval; a gap starts a
    /// new one, so an outage stays visible as two rows.
    #[tokio::test]
    async fn coverage_merges_contiguous_ticks_but_keeps_gaps() {
        let pool = connect_in_memory().await;
        for (start, end) in [
            ("2026-09-01T00:00:00.000Z", "2026-09-01T00:00:10.000Z"),
            ("2026-09-01T00:00:10.000Z", "2026-09-01T00:00:20.000Z"), // touching
            ("2026-09-01T00:00:15.000Z", "2026-09-01T00:00:30.000Z"), // overlapping
            ("2026-09-01T01:00:00.000Z", "2026-09-01T01:00:10.000Z"), // after a gap
        ] {
            record(&pool, &observed(start, end, "complete")).await;
        }
        let rows = coverage_rows(&pool).await;
        assert_eq!(rows.len(), 2, "one merged stretch plus one after the gap");
        assert_eq!(rows[0].0, "2026-09-01T00:00:00.000Z");
        assert_eq!(rows[0].1, "2026-09-01T00:00:30.000Z");
        assert_eq!(rows[1].0, "2026-09-01T01:00:00.000Z");
    }

    /// A merge may widen the START too (a backfill that reaches earlier).
    #[tokio::test]
    async fn coverage_merge_widens_both_ends() {
        let pool = connect_in_memory().await;
        record(
            &pool,
            &observed(
                "2026-09-01T10:00:00.000Z",
                "2026-09-01T11:00:00.000Z",
                "complete",
            ),
        )
        .await;
        record(
            &pool,
            &observed(
                "2026-09-01T09:00:00.000Z",
                "2026-09-01T10:30:00.000Z",
                "complete",
            ),
        )
        .await;
        let rows = coverage_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].0.as_str(), rows[0].1.as_str()),
            ("2026-09-01T09:00:00.000Z", "2026-09-01T11:00:00.000Z")
        );
    }

    /// Different state, scope or diagnostic never merge — a partial stretch
    /// must not vanish inside a complete one.
    #[tokio::test]
    async fn coverage_never_merges_across_state_scope_or_diagnostic() {
        let pool = connect_in_memory().await;
        let base = observed(
            "2026-09-01T00:00:00.000Z",
            "2026-09-01T01:00:00.000Z",
            "complete",
        );
        record(&pool, &base).await;
        record(
            &pool,
            &observed(
                "2026-09-01T01:00:00.000Z",
                "2026-09-01T02:00:00.000Z",
                "partial",
            ),
        )
        .await;
        let mut other_scope = base.clone();
        other_scope.workspace_agent_id = Some("wa");
        record(&pool, &other_scope).await;
        let mut unsupported = base.clone();
        unsupported.state = "partial";
        unsupported.diagnostic_code = Some(UNSUPPORTED_SOURCE);
        record(&pool, &unsupported).await;

        assert_eq!(coverage_rows(&pool).await.len(), 4);
    }

    /// The plan's crash rule: events, coverage and the cursor advance commit
    /// together or not at all, so a crash mid-batch replays cleanly and can
    /// never leave a cursor claiming bytes whose events were lost.
    #[tokio::test]
    async fn events_coverage_and_cursor_commit_atomically() {
        let pool = connect_in_memory().await;

        // Rolled back: nothing of the batch survives.
        {
            let mut tx = pool.begin().await.unwrap();
            insert_event(&mut *tx, &event("k1", "2026-09-05T10:00:00.000Z"))
                .await
                .unwrap();
            record_coverage(
                &mut tx,
                &observed(
                    "2026-09-05T00:00:00.000Z",
                    "2026-09-05T11:00:00.000Z",
                    "partial",
                ),
            )
            .await
            .unwrap();
            upsert_cursor(&mut *tx, &cursor(0, 500)).await.unwrap();
            tx.rollback().await.unwrap();
        }
        assert_eq!(agg(&pool, &UsageScope::default()).await.activity_count, 0);
        assert!(coverage_rows(&pool).await.is_empty());
        assert!(get_cursor(&pool, "claude-code", "fp-1")
            .await
            .unwrap()
            .is_none());

        // Replay of the same batch after the "crash": one activity, one
        // cursor, one interval.
        for _ in 0..2 {
            let mut tx = pool.begin().await.unwrap();
            insert_event(&mut *tx, &event("k1", "2026-09-05T10:00:00.000Z"))
                .await
                .unwrap();
            record_coverage(
                &mut tx,
                &observed(
                    "2026-09-05T00:00:00.000Z",
                    "2026-09-05T11:00:00.000Z",
                    "partial",
                ),
            )
            .await
            .unwrap();
            upsert_cursor(&mut *tx, &cursor(500, 500)).await.unwrap();
            tx.commit().await.unwrap();
        }
        assert_eq!(agg(&pool, &UsageScope::default()).await.activity_count, 1);
        assert_eq!(coverage_rows(&pool).await.len(), 1);
        assert_eq!(
            get_cursor(&pool, "claude-code", "fp-1")
                .await
                .unwrap()
                .unwrap()
                .byte_offset,
            500
        );
    }

    // ── Review ab722021 regressions ──────────────────────────────────────

    /// Two model-less events from different providers are ONE unknown
    /// identity: the exact reproduction was two rows of activity=1/tokens=30
    /// where the wire can only carry one `unknown::` key.
    #[tokio::test]
    async fn model_less_events_group_as_one_unknown_identity_across_providers() {
        let pool = connect_in_memory().await;
        for (key, provider) in [("a", "anthropic"), ("b", "openai")] {
            let mut e = event(key, "2026-09-05T10:00:00Z");
            e.provider = Some(provider.into());
            e.served_model = None;
            e.requested_model = None;
            e.input_tokens = Some(20);
            e.output_tokens = Some(10);
            insert_event(&pool, &e).await.unwrap();
        }
        let groups = aggregate_by_model(&pool, &UsageScope::default(), RANGE_START, RANGE_END)
            .await
            .unwrap();
        assert_eq!(
            groups.len(),
            1,
            "one unknown identity, not one per provider"
        );
        assert_eq!(groups[0].provider, None);
        assert_eq!(groups[0].served_model, None);
        assert_eq!(groups[0].requested_model, None);
        assert_eq!(groups[0].aggregate.activity_count, 2);
        assert_eq!(groups[0].aggregate.measured_tokens, Some(60));
    }

    /// Counters above the ceiling are stored as UNKNOWN with a diagnostic, so
    /// a grouped SUM can neither overflow nor promote to a float. The exact
    /// reproduction: two rows of 2^62 made `SUM` fail with integer overflow.
    #[tokio::test]
    async fn implausible_counters_become_unknown_instead_of_overflowing() {
        let pool = connect_in_memory().await;
        for key in ["a", "b"] {
            let mut e = event(key, "2026-09-05T10:00:00Z");
            e.input_tokens = Some(1 << 62);
            e.output_tokens = Some(1 << 62);
            e.cache_read_input_tokens = Some(i64::MAX);
            insert_event(&pool, &e).await.unwrap();
        }
        // One valid row proves the good side of the ceiling is still counted.
        let mut ok = event("ok", "2026-09-05T10:00:00Z");
        ok.input_tokens = Some(MAX_TOKEN_COUNTER);
        ok.output_tokens = Some(MAX_TOKEN_COUNTER);
        insert_event(&pool, &ok).await.unwrap();
        // A negative counter is equally not a measurement.
        let mut negative = event("neg", "2026-09-05T10:00:00Z");
        negative.output_tokens = Some(-1);
        insert_event(&pool, &negative).await.unwrap();

        let total = agg(&pool, &UsageScope::default()).await;
        assert_eq!(total.activity_count, 4, "the responses still happened");
        assert_eq!(total.measured_event_count, 1);
        assert_eq!(total.unknown_usage_count, 3);
        assert_eq!(total.measured_tokens, Some(2 * MAX_TOKEN_COUNTER));
        // `negative` keeps its known input: partial, not unknown.
        assert_eq!(total.input_tokens, Some(MAX_TOKEN_COUNTER + 100));

        let diagnostics: Vec<(String, Option<String>, Option<i64>)> = sqlx::query_as(
            "SELECT event_key, diagnostic_code, cache_read_input_tokens
               FROM model_usage_event WHERE event_key IN ('a', 'neg') ORDER BY event_key",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(diagnostics[0].1.as_deref(), Some(COUNTER_OUT_OF_RANGE));
        assert_eq!(diagnostics[0].2, None, "the subset counter was dropped too");
        assert_eq!(diagnostics[1].1.as_deref(), Some(COUNTER_OUT_OF_RANGE));
    }

    /// The schema enforces the same ceiling, so a writer that bypasses
    /// `insert_event` cannot smuggle an overflowing counter in either.
    #[tokio::test]
    async fn schema_rejects_counters_above_the_ceiling() {
        let pool = connect_in_memory().await;
        let result = sqlx::query(
            "INSERT INTO model_usage_event (id, event_key, source_kind, source_version,
                event_kind, occurred_at, recorded_at, input_tokens, token_completeness)
             VALUES ('x', 'x', 's', 'v1', 'response', '2026-09-05T10:00:00.000Z',
                     '2026-09-05T10:00:00.000Z', ?1, 'partial')",
        )
        .bind(MAX_TOKEN_COUNTER + 1)
        .execute(&pool)
        .await;
        assert!(result.is_err(), "CHECK must reject a counter above 2^40");
    }

    /// Timestamps reach storage in the one canonical shape whatever the
    /// caller's precision, so lexicographic bucket comparison stays exact.
    #[tokio::test]
    async fn timestamps_are_stored_canonically() {
        let pool = connect_in_memory().await;
        let e = event("k", "2026-09-05T10:00:00+07:00"); // offset, no fraction
        insert_event(&pool, &e).await.unwrap();
        let stored: (String, String) =
            sqlx::query_as("SELECT occurred_at, recorded_at FROM model_usage_event")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored.0, "2026-09-05T03:00:00.000Z");
        assert_eq!(stored.1, "2026-09-05T00:00:00.000Z");
    }

    /// repo/mod.rs contract: serializable rows emit camelCase keys.
    #[test]
    fn coverage_interval_row_camel_case_contract() {
        let row = CoverageIntervalRow {
            id: "id".into(),
            workspace_id: Some("ws".into()),
            workspace_agent_id: None,
            source_kind: "claude-code".into(),
            interval_start: "2026-09-01T00:00:00.000Z".into(),
            interval_end: "2026-09-02T00:00:00.000Z".into(),
            state: "complete".into(),
            collector_version: "v1".into(),
            diagnostic_code: Some(UNSUPPORTED_SOURCE.into()),
            last_verified_at: "2026-09-02T00:00:00.000Z".into(),
        };
        let json = serde_json::to_value(&row).unwrap();
        // serde_json's map is sorted, so compare key SETS.
        let keys: std::collections::BTreeSet<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from([
                "id",
                "workspaceId",
                "workspaceAgentId",
                "sourceKind",
                "intervalStart",
                "intervalEnd",
                "state",
                "collectorVersion",
                "diagnosticCode",
                "lastVerifiedAt",
            ])
        );
        assert!(
            json["workspaceAgentId"].is_null(),
            "None serializes as null"
        );
    }

    // ── Review 34201f49 regressions ──────────────────────────────────────

    /// Exact at the boundary, unavailable one past it — never rounded.
    #[test]
    fn recombine_is_exact_up_to_i64_max_and_unavailable_beyond() {
        assert_eq!(recombine(None, None), (None, false), "no contributing row");
        assert_eq!(recombine(Some(0), Some(0)), (Some(0), false));
        assert_eq!(recombine(Some(3), Some(7)), (Some(3 * SPLIT + 7), false));

        // i64::MAX = hi * 2^20 + lo with hi = 2^43 - 1, lo = 2^20 - 1.
        let hi = (1i64 << 43) - 1;
        let lo = SPLIT - 1;
        assert_eq!(recombine(Some(hi), Some(lo)), (Some(i64::MAX), false));
        // One more than i64::MAX: exactly the 4,194,304-row reproduction
        // (2^22 rows × 2^41 = 2^63 → hi = 2^43, lo = 0).
        assert_eq!(recombine(Some(1i64 << 43), Some(0)), (None, true));
        assert_eq!(recombine(Some(hi + 1), Some(lo)), (None, true));
    }

    /// The reviewer's exact reproduction, end to end through the pinned
    /// projection: 4,194,304 schema-valid rows at the ceiling. Plain `SUM`
    /// raised "integer overflow" here; the split projection returns the
    /// activity and reports the tokens as unavailable. Inserts ~4M rows (about
    /// 30 s and 1.6 GB RSS measured on 2026-09-05), so it is `#[ignore]` and
    /// run as its own recorded gate:
    /// `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored aggregate_at_the_real_overflow_threshold`.
    #[tokio::test]
    #[ignore]
    async fn aggregate_at_the_real_overflow_threshold_is_unavailable_not_an_error() {
        let pool = connect_in_memory().await;
        let rows: i64 = 1 << 22;
        sqlx::query(
            "INSERT INTO model_usage_event (id, event_key, source_kind, source_version,
                event_kind, occurred_at, recorded_at, input_tokens, output_tokens,
                token_completeness, validity)
             WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < ?1)
             SELECT 'id' || x, 'k' || x, 'claude-code', 'v1', 'response',
                    '2026-09-05T10:00:00.000Z', '2026-09-05T10:00:00.000Z',
                    ?2, ?2, 'known', 'valid'
               FROM n",
        )
        .bind(rows)
        .bind(MAX_TOKEN_COUNTER)
        .execute(&pool)
        .await
        .unwrap();

        let total = agg(&pool, &UsageScope::default()).await;
        assert_eq!(total.activity_count, rows);
        assert_eq!(total.measured_event_count, rows);
        assert_eq!(
            total.measured_tokens, None,
            "2^63 does not fit; unavailable"
        );
        assert!(total.measured_overflow);
        // The components individually DO fit (2^22 × 2^40 = 2^62) and stay
        // exact: known finite sums are preserved.
        assert_eq!(total.input_tokens, Some(1i64 << 62));
        assert_eq!(total.output_tokens, Some(1i64 << 62));
        assert!(!total.input_overflow && !total.output_overflow);
    }

    /// 0032 evidence is bounded at the repo the same way the schema bounds it:
    /// an oversized stop reason is dropped (never truncated) with its own
    /// diagnostic, an out-of-range uncached counter is dropped as a counter
    /// rejection, and a counter rejection outranks the stop-reason code.
    #[tokio::test]
    async fn reconciliation_evidence_is_bounded_not_truncated() {
        let pool = connect_in_memory().await;
        let mut exact = event("exact", "2026-09-05T10:00:00Z");
        exact.stop_reason = Some("s".repeat(MAX_STOP_REASON_CHARS));
        exact.source_uncached_input_tokens = Some(MAX_TOKEN_COUNTER);
        insert_event(&pool, &exact).await.unwrap();
        let mut long_stop = event("long_stop", "2026-09-05T10:00:00Z");
        long_stop.stop_reason = Some("s".repeat(MAX_STOP_REASON_CHARS + 1));
        insert_event(&pool, &long_stop).await.unwrap();
        let mut big_uncached = event("big_uncached", "2026-09-05T10:00:00Z");
        big_uncached.source_uncached_input_tokens = Some(MAX_TOKEN_COUNTER + 1);
        insert_event(&pool, &big_uncached).await.unwrap();
        let mut both = event("both", "2026-09-05T10:00:00Z");
        both.stop_reason = Some("s".repeat(MAX_STOP_REASON_CHARS + 1));
        both.source_uncached_input_tokens = Some(-1);
        insert_event(&pool, &both).await.unwrap();

        type EvidenceRow = (String, Option<String>, Option<i64>, Option<String>);
        let rows: Vec<EvidenceRow> = sqlx::query_as(
            "SELECT event_key, stop_reason, source_uncached_input_tokens, diagnostic_code
               FROM model_usage_event ORDER BY event_key",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "big_uncached".into(),
                    None,
                    None,
                    Some(COUNTER_OUT_OF_RANGE.into())
                ),
                ("both".into(), None, None, Some(COUNTER_OUT_OF_RANGE.into())),
                (
                    "exact".into(),
                    Some("s".repeat(MAX_STOP_REASON_CHARS)),
                    Some(MAX_TOKEN_COUNTER),
                    None
                ),
                (
                    "long_stop".into(),
                    None,
                    None,
                    Some(STOP_REASON_OUT_OF_RANGE.into())
                ),
            ]
        );
        // The measured totals are untouched by evidence rejection: the
        // stop-reason code is not a counter rejection.
        let total = agg(&pool, &UsageScope::default()).await;
        assert_eq!(total.rejected_counter_count, 2);
        assert_eq!(total.activity_count, 4);
    }

    /// Rejected counters and conflicts are counted so the reader can cap the
    /// group's coverage; an ordinary unknown-usage row is neither. A rejection
    /// is counted even when the collector had stamped its own diagnostic.
    #[tokio::test]
    async fn damaged_observations_are_counted_separately_from_ordinary_unknowns() {
        let pool = connect_in_memory().await;
        let mut rejected = event("rejected", "2026-09-05T10:00:00Z");
        rejected.input_tokens = Some(-5);
        rejected.diagnostic_code = Some("collector_note".into());
        insert_event(&pool, &rejected).await.unwrap();
        insert_event(&pool, &event("conflicted", "2026-09-05T10:00:00Z"))
            .await
            .unwrap();
        mark_conflict(&pool, "conflicted", "claude_group_disagrees")
            .await
            .unwrap();
        let mut blind = event("blind", "2026-09-05T10:00:00Z");
        blind.input_tokens = None;
        blind.output_tokens = None;
        insert_event(&pool, &blind).await.unwrap();

        let total = agg(&pool, &UsageScope::default()).await;
        assert_eq!(total.activity_count, 3);
        assert_eq!(total.rejected_counter_count, 1);
        assert_eq!(total.conflict_count, 1);
        assert_eq!(total.unknown_usage_count, 3);
        assert_eq!(total.measured_tokens, None);
        assert_eq!(
            total.output_tokens,
            Some(20),
            "the rejected row's good output stays known"
        );
        assert!(!total.measured_overflow);
        let stored: Option<String> = sqlx::query_scalar(
            "SELECT diagnostic_code FROM model_usage_event WHERE event_key = 'rejected'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            stored.as_deref(),
            Some(COUNTER_OUT_OF_RANGE),
            "the rejection code takes precedence over the collector's note"
        );
    }
}

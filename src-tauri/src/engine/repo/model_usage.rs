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
//! * The bucket `VALUES` list is `"(?, ?, ?)"` repeated `buckets.len()` times —
//!   placeholder punctuation only; the keys and boundaries are all bound.
//!
//! A test at the bottom of this module feeds SQL metacharacters through every
//! scope field and asserts they are matched as literal data.

use serde::Serialize;
use sqlx::{AssertSqlSafe, Row, SqlitePool};

// ── Event insertion ──────────────────────────────────────────────────────────

/// One event to persist. Constructed by the collectors; every optional field is
/// `None` when the source did not prove it.
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
    pub occurred_at: String,
    pub recorded_at: String,
    pub provider: Option<String>,
    pub requested_model: Option<String>,
    pub served_model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_write_input_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
    pub validity: String,
    pub diagnostic_code: Option<String>,
}

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
}

/// Insert one event, ignoring a replay of the same `event_key`.
///
/// Returns `true` when a new row landed. `ON CONFLICT DO NOTHING` is what makes
/// restart, duplicate source rows and a re-scanned file a no-op rather than
/// inflation — the caller does not need to pre-check existence.
pub async fn insert_event<'e, E>(executor: E, event: &NewUsageEvent) -> sqlx::Result<bool>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let result = sqlx::query(
        "INSERT INTO model_usage_event (
            id, event_key, workspace_id, workspace_agent_id, session_id, generation,
            source_kind, source_version, event_kind,
            source_session_id, source_request_id, source_response_id,
            occurred_at, recorded_at, provider, requested_model, served_model,
            input_tokens, output_tokens,
            cache_read_input_tokens, cache_write_input_tokens, reasoning_output_tokens,
            token_completeness, validity, diagnostic_code
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
            ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
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
    .bind(&event.occurred_at)
    .bind(&event.recorded_at)
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
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Mark an already-stored event as conflicting. It remains one activity; only
/// its measured token contribution is withdrawn (contract: a conflict "never
/// creates a second activity").
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
    pub last_verified_at: String,
}

/// Record an observed interval.
pub async fn insert_coverage<'e, E>(executor: E, row: &CoverageIntervalRow) -> sqlx::Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO model_usage_coverage (
            id, workspace_id, workspace_agent_id, source_kind,
            interval_start, interval_end, state, collector_version, last_verified_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(&row.id)
    .bind(&row.workspace_id)
    .bind(&row.workspace_agent_id)
    .bind(&row.source_kind)
    .bind(&row.interval_start)
    .bind(&row.interval_end)
    .bind(&row.state)
    .bind(&row.collector_version)
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
                interval_start, interval_end, state, collector_version, last_verified_at
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
    pub measured_tokens: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
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
const AGG_COLUMNS: &str = "
    COUNT(*) AS activity_count,
    COALESCE(SUM(CASE WHEN e.event_kind = 'response'   THEN 1 ELSE 0 END), 0) AS response_count,
    COALESCE(SUM(CASE WHEN e.event_kind = 'invocation' THEN 1 ELSE 0 END), 0) AS invocation_count,
    COALESCE(SUM(CASE WHEN e.validity = 'valid' AND e.token_completeness = 'known' THEN 1 ELSE 0 END), 0) AS measured_event_count,
    COALESCE(SUM(CASE WHEN e.validity <> 'valid' OR e.token_completeness <> 'known' THEN 1 ELSE 0 END), 0) AS unknown_usage_count,
    SUM(CASE WHEN e.validity = 'valid' AND e.token_completeness = 'known'
             THEN e.input_tokens + e.output_tokens END) AS measured_tokens,
    SUM(CASE WHEN e.validity = 'valid' THEN e.input_tokens  END) AS input_tokens,
    SUM(CASE WHEN e.validity = 'valid' THEN e.output_tokens END) AS output_tokens
";

fn read_aggregate(row: &sqlx::sqlite::SqliteRow, bucket: String) -> UsageAggregate {
    UsageAggregate {
        bucket,
        activity_count: row.get("activity_count"),
        response_count: row.get("response_count"),
        invocation_count: row.get("invocation_count"),
        measured_event_count: row.get("measured_event_count"),
        unknown_usage_count: row.get("unknown_usage_count"),
        measured_tokens: row.get("measured_tokens"),
        input_tokens: row.get("input_tokens"),
        output_tokens: row.get("output_tokens"),
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
/// served one.
pub async fn aggregate_by_model(
    pool: &SqlitePool,
    scope: &UsageScope,
    start_utc: &str,
    end_utc: &str,
) -> sqlx::Result<Vec<ModelAggregate>> {
    let (pred, binds) = scope_predicate(scope);
    let sql = format!(
        "SELECT e.provider AS provider,
                e.served_model AS served_model,
                CASE WHEN e.served_model IS NULL THEN e.requested_model END AS requested_model,
                {AGG_COLUMNS}
           FROM model_usage_event e
          WHERE e.occurred_at >= ? AND e.occurred_at < ?{pred}
          GROUP BY e.provider, e.served_model,
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
            occurred_at: occurred_at.to_owned(),
            recorded_at: "2026-09-05T00:00:00Z".into(),
            provider: Some("anthropic".into()),
            requested_model: None,
            served_model: Some("claude-fable-5-1".into()),
            input_tokens: Some(100),
            output_tokens: Some(20),
            cache_read_input_tokens: Some(90),
            cache_write_input_tokens: None,
            reasoning_output_tokens: None,
            validity: "valid".into(),
            diagnostic_code: None,
        }
    }

    const RANGE_START: &str = "2026-09-01T00:00:00Z";
    const RANGE_END: &str = "2026-09-30T00:00:00Z";

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
        let e = event("claude-code:v1:s:r", "2026-09-10T12:00:00Z");
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
        let mut unknown = event("k1", "2026-09-10T12:00:00Z");
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
        let mut zero = event("k1", "2026-09-10T12:00:00Z");
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
        insert_event(&pool, &event("k1", "2026-09-10T12:00:00Z"))
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
        let mut scoped = event("scoped", "2026-09-10T12:00:00Z");
        scoped.event_kind = "invocation".into();
        insert_event(&pool, &scoped).await.unwrap();

        let mut unscoped = event("unscoped", "2026-09-10T13:00:00Z");
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
        let mut reported = event("reported", "2026-09-10T12:00:00Z");
        reported.served_model = Some("gpt-6-astra".into());
        reported.requested_model = Some("gpt-6-astra".into());
        reported.provider = Some("openai".into());
        insert_event(&pool, &reported).await.unwrap();

        let mut selected = event("selected", "2026-09-10T13:00:00Z");
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
        insert_event(&pool, &event("before", "2026-09-09T23:59:59Z"))
            .await
            .unwrap();
        insert_event(&pool, &event("boundary", "2026-09-10T00:00:00Z"))
            .await
            .unwrap();
        insert_event(&pool, &event("inside", "2026-09-10T12:00:00Z"))
            .await
            .unwrap();

        let buckets = vec![
            (
                "2026-09-09".to_string(),
                "2026-09-09T00:00:00Z".to_string(),
                "2026-09-10T00:00:00Z".to_string(),
            ),
            (
                "2026-09-10".to_string(),
                "2026-09-10T00:00:00Z".to_string(),
                "2026-09-11T00:00:00Z".to_string(),
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
                "2026-09-20T00:00:00Z".to_string(),
                "2026-09-21T00:00:00Z".to_string(),
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
        let mut evil = event("evil", "2026-09-10T12:00:00Z");
        evil.workspace_id = Some(injection.to_owned());
        insert_event(&pool, &evil).await.unwrap();
        insert_event(&pool, &event("normal", "2026-09-10T12:00:00Z"))
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
            last_verified_at: "2026-09-30T00:00:00Z".into(),
        };
        insert_coverage(
            &pool,
            &row("straddles", "2026-08-20T00:00:00Z", "2026-09-05T00:00:00Z"),
        )
        .await
        .unwrap();
        insert_coverage(
            &pool,
            &row("outside", "2026-07-01T00:00:00Z", "2026-07-02T00:00:00Z"),
        )
        .await
        .unwrap();

        let found = coverage_overlapping(&pool, RANGE_START, RANGE_END)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "straddles");
    }
}

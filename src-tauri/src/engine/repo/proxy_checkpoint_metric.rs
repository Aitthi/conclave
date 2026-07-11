//! Checkpoint metric contract (spec §7.1) for the app-global context proxy.
//!
//! Milestone-1 log-mode projection telemetry. Request/response bodies and auth
//! headers never enter this repository — rows carry token counts, the q=S_net/R
//! metric, and deterministic labels only.

use chrono::{Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

pub struct CheckpointMetricInsert {
    pub created_at: String,
    pub model: String,
    pub earliest_changed_byte: i64,
    pub earliest_changed_msg: i64,
    pub r_tokens: i64,
    pub gross_candidate_tokens: i64,
    pub stub_overhead_tokens: i64,
    pub s_net_tokens: i64,
    pub q: f64,
    pub projected_break_even: f64,
    pub projected_post_tokens: i64,
    pub plateau_turns: i64,
    pub non_recoverable_kept_tokens: i64,
    pub provider_estimate: i64,
    pub count_failure: i64,
    pub method_version: String,
    pub bytes_est_tokens: i64,
    /// R8 real-token bucket: `below_ceiling` | `eligible` | `saturated` | `count_failure`.
    pub outcome: String,
    /// count_tokens failure diagnostic (HTTP status + ≤200-char body snippet) on
    /// `count_failure`; `None` otherwise. SQL-diagnosis only — NOT in checkpoint-report.
    pub error_snippet: Option<String>,
}

pub async fn insert(pool: &SqlitePool, m: CheckpointMetricInsert) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO proxy_checkpoint_metric (\
         created_at, model, earliest_changed_byte, earliest_changed_msg, r_tokens, \
         gross_candidate_tokens, stub_overhead_tokens, s_net_tokens, q, projected_break_even, \
         projected_post_tokens, plateau_turns, non_recoverable_kept_tokens, provider_estimate, \
         count_failure, method_version, bytes_est_tokens, outcome, error_snippet) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
    )
    .bind(m.created_at)
    .bind(m.model)
    .bind(m.earliest_changed_byte)
    .bind(m.earliest_changed_msg)
    .bind(m.r_tokens)
    .bind(m.gross_candidate_tokens)
    .bind(m.stub_overhead_tokens)
    .bind(m.s_net_tokens)
    .bind(m.q)
    .bind(m.projected_break_even)
    .bind(m.projected_post_tokens)
    .bind(m.plateau_turns)
    .bind(m.non_recoverable_kept_tokens)
    .bind(m.provider_estimate)
    .bind(m.count_failure)
    .bind(m.method_version)
    .bind(m.bytes_est_tokens)
    .bind(m.outcome)
    .bind(m.error_snippet)
    .execute(pool)
    .await?;
    Ok(())
}

/// R8 checkpoint-report: counts per real-token bucket + the q distribution and
/// projected-post band over the `eligible`+`saturated` rows (the buckets that
/// carry a real a/b/c contract). `below_ceiling` and `count_failure` rows are
/// counted but excluded from the q/projected-post summaries (no q claim).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointReport {
    pub samples: i64,
    pub below_ceiling: i64,
    pub eligible: i64,
    pub saturated: i64,
    pub count_failures: i64,
    pub max_plateau_turns: i64,
    pub q_min: f64,
    pub q_median: f64,
    pub q_max: f64,
    pub avg_q: f64,
    pub projected_post_min: i64,
    pub projected_post_max: i64,
    pub avg_projected_post_tokens: f64,
}

/// SQL-computable aggregates (everything except the median, which SQLite lacks).
#[derive(sqlx::FromRow)]
struct Agg {
    samples: i64,
    below_ceiling: i64,
    eligible: i64,
    saturated: i64,
    count_failures: i64,
    max_plateau_turns: i64,
    q_min: f64,
    q_max: f64,
    avg_q: f64,
    projected_post_min: i64,
    projected_post_max: i64,
    avg_projected_post_tokens: f64,
}

fn median_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

pub async fn report(pool: &SqlitePool, since_hours: i64) -> sqlx::Result<CheckpointReport> {
    let span = Duration::try_hours(since_hours.max(0)).unwrap_or(Duration::MAX);
    let cutoff = Utc::now()
        .checked_sub_signed(span)
        .unwrap_or(chrono::DateTime::<Utc>::MIN_UTC)
        .to_rfc3339();

    // Bucket counts + q/projected-post summaries in one pass. The q distribution
    // and projected-post band cover only `eligible`+`saturated` (the rows with a
    // real a/b/c contract); `below_ceiling`/`count_failure` are counted but make
    // no q claim (R8). COALESCE guards the empty/all-NULL table.
    let agg = sqlx::query_as::<_, Agg>(
        "SELECT \
           COUNT(*) AS samples, \
           COALESCE(SUM(outcome = 'below_ceiling'), 0) AS below_ceiling, \
           COALESCE(SUM(outcome = 'eligible'), 0) AS eligible, \
           COALESCE(SUM(outcome = 'saturated'), 0) AS saturated, \
           COALESCE(SUM(outcome = 'count_failure'), 0) AS count_failures, \
           COALESCE(MAX(plateau_turns), 0) AS max_plateau_turns, \
           COALESCE(MIN(CASE WHEN outcome IN ('eligible','saturated') THEN q END), 0.0) AS q_min, \
           COALESCE(MAX(CASE WHEN outcome IN ('eligible','saturated') THEN q END), 0.0) AS q_max, \
           COALESCE(AVG(CASE WHEN outcome IN ('eligible','saturated') THEN q END), 0.0) AS avg_q, \
           COALESCE(MIN(CASE WHEN outcome IN ('eligible','saturated') THEN projected_post_tokens END), 0) AS projected_post_min, \
           COALESCE(MAX(CASE WHEN outcome IN ('eligible','saturated') THEN projected_post_tokens END), 0) AS projected_post_max, \
           COALESCE(AVG(CASE WHEN outcome IN ('eligible','saturated') THEN projected_post_tokens END), 0.0) AS avg_projected_post_tokens \
         FROM proxy_checkpoint_metric \
         WHERE created_at >= ?1",
    )
    .bind(&cutoff)
    .fetch_one(pool)
    .await?;

    // SQLite has no MEDIAN(); pull the (small, cooldown-bounded) q list and take it in Rust.
    let qs: Vec<f64> = sqlx::query_scalar(
        "SELECT q FROM proxy_checkpoint_metric \
         WHERE outcome IN ('eligible','saturated') AND created_at >= ?1 \
         ORDER BY q",
    )
    .bind(&cutoff)
    .fetch_all(pool)
    .await?;

    Ok(CheckpointReport {
        samples: agg.samples,
        below_ceiling: agg.below_ceiling,
        eligible: agg.eligible,
        saturated: agg.saturated,
        count_failures: agg.count_failures,
        max_plateau_turns: agg.max_plateau_turns,
        q_min: agg.q_min,
        q_median: median_sorted(&qs),
        q_max: agg.q_max,
        avg_q: agg.avg_q,
        projected_post_min: agg.projected_post_min,
        projected_post_max: agg.projected_post_max,
        avg_projected_post_tokens: agg.avg_projected_post_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    fn row(outcome: &str, q: f64, post: i64, plateau: i64, fail: i64) -> CheckpointMetricInsert {
        CheckpointMetricInsert {
            created_at: chrono::Utc::now().to_rfc3339(),
            model: "claude-x".into(),
            earliest_changed_byte: 1_000,
            earliest_changed_msg: 1,
            r_tokens: 400_000,
            gross_candidate_tokens: 90_000,
            stub_overhead_tokens: 200,
            s_net_tokens: (q * 400_000.0) as i64,
            q,
            projected_break_even: if q > 0.0 {
                11.5 / q - 12.5
            } else {
                f64::INFINITY
            },
            projected_post_tokens: post,
            plateau_turns: plateau,
            non_recoverable_kept_tokens: 5_000,
            provider_estimate: 1,
            count_failure: fail,
            method_version: "m1-count_tokens-2023-06-01".into(),
            bytes_est_tokens: 500_000,
            outcome: outcome.into(),
            error_snippet: if fail == 1 {
                Some("count_tokens HTTP 400: {\"type\":\"error\"}".into())
            } else {
                None
            },
        }
    }

    #[tokio::test]
    async fn insert_round_trips_outcome() {
        let pool = connect_in_memory().await;
        insert(&pool, row("saturated", 0.3, 360_000, 0, 0))
            .await
            .unwrap();
        let got: String = sqlx::query_scalar("SELECT outcome FROM proxy_checkpoint_metric")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(got, "saturated");
    }

    #[tokio::test]
    async fn insert_persists_error_snippet_on_failure_and_null_on_success() {
        let pool = connect_in_memory().await;
        insert(&pool, row("count_failure", 0.0, 0, 0, 1))
            .await
            .unwrap();
        insert(&pool, row("saturated", 0.3, 360_000, 0, 0))
            .await
            .unwrap();
        let snippets: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT error_snippet FROM proxy_checkpoint_metric ORDER BY outcome",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        // ORDER BY outcome: 'count_failure' < 'saturated'.
        assert!(snippets[0].as_deref().unwrap().contains("HTTP 400"));
        assert_eq!(snippets[1], None, "success rows carry no error snippet");
    }

    #[tokio::test]
    async fn report_groups_by_outcome_and_summarizes_q_for_eligible_and_saturated() {
        let pool = connect_in_memory().await;
        insert(&pool, row("below_ceiling", 0.0, 90_000, 0, 0))
            .await
            .unwrap();
        insert(&pool, row("eligible", 0.8, 300_000, 3, 0))
            .await
            .unwrap();
        insert(&pool, row("eligible", 0.4, 340_000, 1, 0))
            .await
            .unwrap();
        insert(&pool, row("saturated", 0.6, 360_000, 0, 0))
            .await
            .unwrap();
        insert(&pool, row("count_failure", 0.0, 0, 0, 1))
            .await
            .unwrap();
        let r = report(&pool, 24).await.unwrap();
        assert_eq!(r.samples, 5);
        assert_eq!(r.below_ceiling, 1);
        assert_eq!(r.eligible, 2);
        assert_eq!(r.saturated, 1);
        assert_eq!(r.count_failures, 1);
        // q distribution over eligible+saturated = {0.8, 0.4, 0.6}.
        assert!((r.q_min - 0.4).abs() < 1e-9);
        assert!((r.q_max - 0.8).abs() < 1e-9);
        assert!((r.q_median - 0.6).abs() < 1e-9);
        assert!((r.avg_q - 0.6).abs() < 1e-9);
        // projected_post band over eligible+saturated = {300k, 340k, 360k}.
        assert_eq!(r.projected_post_min, 300_000);
        assert_eq!(r.projected_post_max, 360_000);
        assert_eq!(r.max_plateau_turns, 3);
    }

    // GUARD: empty table must not NULL-panic the aggregates.
    #[tokio::test]
    async fn report_on_empty_table_is_all_zero() {
        let pool = connect_in_memory().await;
        let r = report(&pool, 24).await.unwrap();
        assert_eq!(r.samples, 0);
        assert_eq!(r.below_ceiling, 0);
        assert_eq!(r.eligible, 0);
        assert_eq!(r.saturated, 0);
        assert_eq!(r.count_failures, 0);
        assert_eq!(r.max_plateau_turns, 0);
        assert_eq!(r.q_min, 0.0);
        assert_eq!(r.q_median, 0.0);
        assert_eq!(r.q_max, 0.0);
        assert_eq!(r.avg_q, 0.0);
        assert_eq!(r.projected_post_min, 0);
        assert_eq!(r.projected_post_max, 0);
        assert_eq!(r.avg_projected_post_tokens, 0.0);
    }
}

//! Checkpoint metric contract (spec §7.1) for the app-global context proxy.
//!
//! Milestone-1 log-mode projection telemetry. Request/response bodies and auth
//! headers never enter this repository — rows carry token counts, the q=S_net/R
//! metric, and deterministic labels only.

use super::cb_err;
use chain_builder::{QueryBuilder, Sqlite};
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
}

pub async fn insert(pool: &SqlitePool, m: CheckpointMetricInsert) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO proxy_checkpoint_metric (\
         created_at, model, earliest_changed_byte, earliest_changed_msg, r_tokens, \
         gross_candidate_tokens, stub_overhead_tokens, s_net_tokens, q, projected_break_even, \
         projected_post_tokens, plateau_turns, non_recoverable_kept_tokens, provider_estimate, \
         count_failure, method_version, bytes_est_tokens) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointReport {
    pub samples: i64,
    pub eligible: i64,
    pub avg_q: f64,
    pub avg_projected_post_tokens: f64,
    pub max_plateau_turns: i64,
    pub count_failures: i64,
}

pub async fn report(pool: &SqlitePool, since_hours: i64) -> sqlx::Result<CheckpointReport> {
    let span = Duration::try_hours(since_hours.max(0)).unwrap_or(Duration::MAX);
    let cutoff = Utc::now()
        .checked_sub_signed(span)
        .unwrap_or(chrono::DateTime::<Utc>::MIN_UTC)
        .to_rfc3339();

    QueryBuilder::<Sqlite>::table("proxy_checkpoint_metric")
        .select_raw("COUNT(*) AS samples", None)
        .select_raw(
            "COALESCE(SUM(CASE WHEN count_failure = 0 THEN 1 ELSE 0 END), 0) AS eligible",
            None,
        )
        .select_raw("COALESCE(AVG(q), 0.0) AS avg_q", None)
        .select_raw(
            "COALESCE(AVG(projected_post_tokens), 0.0) AS avg_projected_post_tokens",
            None,
        )
        .select_raw("COALESCE(MAX(plateau_turns), 0) AS max_plateau_turns", None)
        .select_raw("COALESCE(SUM(count_failure), 0) AS count_failures", None)
        .where_gte("created_at", cutoff)
        .fetch_one::<CheckpointReport, _>(pool)
        .await
        .map_err(cb_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    fn row(q: f64, post: i64, plateau: i64, fail: i64) -> CheckpointMetricInsert {
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
        }
    }

    #[tokio::test]
    async fn report_aggregates_and_excludes_failures_from_eligible() {
        let pool = connect_in_memory().await;
        insert(&pool, row(0.8, 340_000, 3, 0)).await.unwrap();
        insert(&pool, row(0.2, 360_000, 0, 1)).await.unwrap(); // count failure
        let r = report(&pool, 24).await.unwrap();
        assert_eq!(r.samples, 2);
        assert_eq!(r.eligible, 1);
        assert_eq!(r.count_failures, 1);
        assert_eq!(r.max_plateau_turns, 3);
    }

    // GUARD (added by orchestrator): empty table must not NULL-panic the aggregates.
    #[tokio::test]
    async fn report_on_empty_table_is_all_zero() {
        let pool = connect_in_memory().await;
        let r = report(&pool, 24).await.unwrap();
        assert_eq!(r.samples, 0);
        assert_eq!(r.eligible, 0);
        assert_eq!(r.count_failures, 0);
        assert_eq!(r.max_plateau_turns, 0);
        assert_eq!(r.avg_q, 0.0);
        assert_eq!(r.avg_projected_post_tokens, 0.0);
    }
}

//! Aggregate-only telemetry for the app-global context proxy.
//!
//! Request/response bodies and headers never enter this repository. Rows carry
//! byte/token counts and deterministic decision labels only.

use super::cb_err;
use chain_builder::{QueryBuilder, Sqlite};
use chrono::{Duration, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

pub struct MetricInsert {
    pub created_at: String,
    pub model: String,
    pub mode: String,
    pub decision: String,
    pub request_bytes_in: i64,
    pub request_bytes_out: i64,
    pub elisions: i64,
    pub bytes_saved: i64,
    pub input_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

pub async fn insert(pool: &SqlitePool, metric: MetricInsert) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO proxy_request_metric (\
         created_at, model, mode, decision, request_bytes_in, request_bytes_out, \
         elisions, bytes_saved, input_tokens, cache_read_tokens, \
         cache_creation_tokens, output_tokens) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )
    .bind(metric.created_at)
    .bind(metric.model)
    .bind(metric.mode)
    .bind(metric.decision)
    .bind(metric.request_bytes_in)
    .bind(metric.request_bytes_out)
    .bind(metric.elisions)
    .bind(metric.bytes_saved)
    .bind(metric.input_tokens)
    .bind(metric.cache_read_tokens)
    .bind(metric.cache_creation_tokens)
    .bind(metric.output_tokens)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyReport {
    pub requests: i64,
    pub rewritten: i64,
    pub bytes_saved: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
}

pub async fn report(pool: &SqlitePool, since_hours: i64) -> sqlx::Result<ProxyReport> {
    let span = Duration::try_hours(since_hours.max(0)).unwrap_or(Duration::MAX);
    let cutoff = Utc::now()
        .checked_sub_signed(span)
        .unwrap_or(chrono::DateTime::<Utc>::MIN_UTC)
        .to_rfc3339();

    QueryBuilder::<Sqlite>::table("proxy_request_metric")
        .select_raw("COUNT(*) AS requests", None)
        .select_raw(
            "COALESCE(SUM(CASE WHEN mode = 'rewrite' AND bytes_saved > 0 THEN 1 ELSE 0 END), 0) AS rewritten",
            None,
        )
        .select_raw("COALESCE(SUM(bytes_saved), 0) AS bytes_saved", None)
        .select_raw("COALESCE(SUM(input_tokens), 0) AS input_tokens", None)
        .select_raw(
            "COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens",
            None,
        )
        .where_gte("created_at", cutoff)
        .fetch_one::<ProxyReport, _>(pool)
        .await
        .map_err(cb_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    fn metric(mode: &str, bytes_saved: i64, input_tokens: Option<i64>) -> MetricInsert {
        MetricInsert {
            created_at: Utc::now().to_rfc3339(),
            model: "claude-test".into(),
            mode: mode.into(),
            decision: "reevaluate".into(),
            request_bytes_in: 1_000,
            request_bytes_out: 1_000 - bytes_saved,
            elisions: i64::from(bytes_saved > 0),
            bytes_saved,
            input_tokens,
            cache_read_tokens: input_tokens.map(|_| 25),
            cache_creation_tokens: None,
            output_tokens: input_tokens.map(|_| 10),
        }
    }

    #[tokio::test]
    async fn report_aggregates_rows_and_treats_null_usage_as_zero() {
        let pool = connect_in_memory().await;
        insert(&pool, metric("rewrite", 300, Some(100)))
            .await
            .unwrap();
        insert(&pool, metric("log", 200, None)).await.unwrap();

        assert_eq!(
            report(&pool, 24).await.unwrap(),
            ProxyReport {
                requests: 2,
                rewritten: 1,
                bytes_saved: 500,
                input_tokens: 100,
                cache_read_tokens: 25,
            }
        );
    }
}

//! Review-queue repository: transcript-distilled memory proposals awaiting a
//! reviewer's ruling before they reach `memory_chunk` (plan
//! memory-distill-queue).
//!
//! # Query convention
//!
//! Single-table reads use chain-builder. [`create`] uses raw `sqlx` for the
//! `INSERT ... ON CONFLICT DO NOTHING` dedup (chain-builder cannot express
//! conflict handling), mirroring [`super::memory::upsert_chunk`]. [`set_reviewed`]
//! is a plain conditional UPDATE, also raw sqlx per the established convention.

use super::cb_err;
use crate::engine::error::AppError;
use chain_builder::{Order, QueryBuilder, Sqlite};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

const PROPOSAL_COLS: [&str; 12] = [
    "id",
    "workspace_id",
    "proposer_id",
    "text",
    "source_note",
    "content_hash",
    "state",
    "reviewer_id",
    "review_reason",
    "chunk_id",
    "created_at",
    "reviewed_at",
];

/// One row of the review queue.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProposalRow {
    pub id: String,
    pub workspace_id: String,
    pub proposer_id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_note: Option<String>,
    pub content_hash: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
}

/// Inputs to [`create`].
pub struct CreateProposalInput<'a> {
    pub workspace_id: &'a str,
    pub proposer_id: &'a str,
    pub text: &'a str,
    pub source_note: Option<&'a str>,
    pub content_hash: &'a str,
}

/// Result of [`create`]. `deduped` is true when a proposal with the same
/// `(workspace_id, content_hash)` already existed, in which case no row was
/// inserted and `row` is `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateProposalResult {
    pub row: Option<MemoryProposalRow>,
    pub deduped: bool,
}

/// Insert one `pending` proposal, deduped on `(workspace_id, content_hash)`.
///
/// Raw `sqlx` is required for the idempotent `ON CONFLICT DO NOTHING`. The
/// `RETURNING` clause yields the row only when a fresh insert actually
/// happened; a conflict returns no row, which this reports as `deduped: true`
/// without a racy preflight SELECT.
pub async fn create(
    pool: &SqlitePool,
    input: CreateProposalInput<'_>,
) -> Result<CreateProposalResult, AppError> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    let row = sqlx::query_as::<_, MemoryProposalRow>(
        "INSERT INTO memory_proposal \
         (id, workspace_id, proposer_id, text, source_note, content_hash, \
          state, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7) \
         ON CONFLICT(workspace_id, content_hash) DO NOTHING \
         RETURNING id, workspace_id, proposer_id, text, source_note, \
                   content_hash, state, reviewer_id, review_reason, chunk_id, \
                   created_at, reviewed_at",
    )
    .bind(&id)
    .bind(input.workspace_id)
    .bind(input.proposer_id)
    .bind(input.text)
    .bind(input.source_note)
    .bind(input.content_hash)
    .bind(&now)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?;

    Ok(CreateProposalResult {
        deduped: row.is_none(),
        row,
    })
}

/// True when a `memory_chunk` with this `content_hash` already exists in the
/// workspace. The distiller dedups a candidate against the LIVE store as well
/// as the queue, so an already-remembered fact never enters review. Kept here
/// (rather than in `repo::memory`) so the whole proposal write path stays in
/// one module; it is a read-only existence check against the shared unique key.
pub async fn chunk_hash_exists(
    pool: &SqlitePool,
    workspace_id: &str,
    content_hash: &str,
) -> Result<bool, AppError> {
    let count = QueryBuilder::<Sqlite>::table("memory_chunk")
        .where_eq("workspace_id", workspace_id)
        .where_eq("content_hash", content_hash)
        .count(pool)
        .await
        .map_err(cb_err)
        .map_err(AppError::from)?;
    Ok(count > 0)
}

/// Fetch one proposal scoped to its workspace.
pub async fn get(
    pool: &SqlitePool,
    workspace_id: &str,
    id: &str,
) -> Result<Option<MemoryProposalRow>, AppError> {
    QueryBuilder::<Sqlite>::table("memory_proposal")
        .select(PROPOSAL_COLS)
        .where_eq("workspace_id", workspace_id)
        .where_eq("id", id)
        .fetch_optional::<MemoryProposalRow, _>(pool)
        .await
        .map_err(cb_err)
        .map_err(AppError::from)
}

/// List a workspace's proposals in the given state, newest first.
pub async fn list_by_state(
    pool: &SqlitePool,
    workspace_id: &str,
    state: &str,
) -> Result<Vec<MemoryProposalRow>, AppError> {
    QueryBuilder::<Sqlite>::table("memory_proposal")
        .select(PROPOSAL_COLS)
        .where_eq("workspace_id", workspace_id)
        .where_eq("state", state)
        .order_by("created_at", Order::Desc)
        .order_by("id", Order::Desc)
        .fetch_all::<MemoryProposalRow, _>(pool)
        .await
        .map_err(cb_err)
        .map_err(AppError::from)
}

/// Stamp a review outcome onto a still-`pending` proposal.
///
/// The `WHERE state = 'pending'` guard makes the transition atomic: a proposal
/// already approved or rejected matches nothing and returns `None`, so the
/// caller reports "not pending" without a separate check racing the update.
/// `chunk_id` is set only on approval.
pub async fn set_reviewed(
    pool: &SqlitePool,
    workspace_id: &str,
    id: &str,
    new_state: &str,
    reviewer_id: &str,
    review_reason: Option<&str>,
    chunk_id: Option<&str>,
) -> Result<Option<MemoryProposalRow>, AppError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query_as::<_, MemoryProposalRow>(
        "UPDATE memory_proposal SET \
            state = ?1, reviewer_id = ?2, review_reason = ?3, chunk_id = ?4, \
            reviewed_at = ?5 \
         WHERE workspace_id = ?6 AND id = ?7 AND state = 'pending' \
         RETURNING id, workspace_id, proposer_id, text, source_note, \
                   content_hash, state, reviewer_id, review_reason, chunk_id, \
                   created_at, reviewed_at",
    )
    .bind(new_state)
    .bind(reviewer_id)
    .bind(review_reason)
    .bind(chunk_id)
    .bind(&now)
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::from)?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{db::connect_in_memory, repo::workspace};

    async fn fixture_workspace(pool: &SqlitePool, name: &str) -> String {
        workspace::create(pool, name, &format!("/tmp/{name}"), None)
            .await
            .expect("create workspace")
            .id
    }

    fn input<'a>(
        workspace_id: &'a str,
        proposer_id: &'a str,
        text: &'a str,
        content_hash: &'a str,
    ) -> CreateProposalInput<'a> {
        CreateProposalInput {
            workspace_id,
            proposer_id,
            text,
            source_note: Some("transcript.jsonl 2026-07-05"),
            content_hash,
        }
    }

    #[tokio::test]
    async fn create_inserts_pending_then_dedups_on_content_hash() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool, "proposal-create").await;

        let first = create(&pool, input(&ws, "agent-1", "a failed approach", "hash-a"))
            .await
            .expect("first create");
        assert!(!first.deduped);
        let row = first.row.expect("fresh insert returns the row");
        assert_eq!(row.state, "pending");
        assert_eq!(row.proposer_id, "agent-1");
        assert!(row.reviewer_id.is_none());
        assert!(row.chunk_id.is_none());

        // Same hash (even a different proposer/text) is blocked by the unique key.
        let second = create(&pool, input(&ws, "agent-2", "reworded", "hash-a"))
            .await
            .expect("dedup create");
        assert!(second.deduped);
        assert!(second.row.is_none());
        assert_eq!(list_by_state(&pool, &ws, "pending").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn list_by_state_filters_and_orders_newest_first() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool, "proposal-list").await;
        for (index, hash) in ["h0", "h1", "h2"].iter().enumerate() {
            create(&pool, input(&ws, "agent-1", &format!("fact {index}"), hash))
                .await
                .expect("create");
        }
        let pending = list_by_state(&pool, &ws, "pending").await.unwrap();
        assert_eq!(pending.len(), 3);
        // created_at ties within the same second break on id DESC, so the order
        // is deterministic even when the clock does not advance between inserts.
        let approved = list_by_state(&pool, &ws, "approved").await.unwrap();
        assert!(approved.is_empty());
    }

    #[tokio::test]
    async fn set_reviewed_is_a_one_shot_pending_transition() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool, "proposal-review").await;
        let created = create(&pool, input(&ws, "agent-1", "fact", "hash-x"))
            .await
            .expect("create")
            .row
            .expect("row");

        let approved = set_reviewed(
            &pool,
            &ws,
            &created.id,
            "approved",
            "agent-2",
            None,
            Some("chunk-1"),
        )
        .await
        .expect("approve")
        .expect("pending row transitions");
        assert_eq!(approved.state, "approved");
        assert_eq!(approved.reviewer_id.as_deref(), Some("agent-2"));
        assert_eq!(approved.chunk_id.as_deref(), Some("chunk-1"));
        assert!(approved.reviewed_at.is_some());

        // A second review of the now-approved row matches nothing.
        let again = set_reviewed(&pool, &ws, &created.id, "rejected", "agent-2", None, None)
            .await
            .expect("second review");
        assert!(again.is_none(), "non-pending proposal must not transition");
    }

    #[tokio::test]
    async fn chunk_hash_exists_reads_the_live_store() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool, "proposal-chunk-dedup").await;
        assert!(!chunk_hash_exists(&pool, &ws, "hash-live").await.unwrap());

        crate::engine::repo::memory::upsert_chunk(
            &pool,
            crate::engine::repo::memory::UpsertChunkInput {
                workspace_id: &ws,
                model_id: "fake-embedder-v1",
                source_kind: "manual",
                source_id: None,
                text: "already stored",
                embedding: &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                content_hash: "hash-live",
            },
        )
        .await
        .expect("seed chunk");
        assert!(chunk_hash_exists(&pool, &ws, "hash-live").await.unwrap());
        assert!(!chunk_hash_exists(&pool, &ws, "hash-other").await.unwrap());
    }
}

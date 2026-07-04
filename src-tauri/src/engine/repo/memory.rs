//! Workspace memory repository.
//!
//! `memory_index` pins one embedding model and dimension to a workspace.
//! `memory_chunk` stores explicit memories plus their normalized little-endian
//! `f32` vectors.
//!
//! # Query convention
//!
//! Single-table SELECTs use chain-builder. [`ensure_index`] and
//! [`upsert_chunk`] use raw `sqlx` because both require SQLite conflict
//! handling (`INSERT ... ON CONFLICT`), which cannot express the conditional
//! validation and `RETURNING` behavior needed here. Plain deletes use the
//! established raw-sqlx repository convention.

use super::cb_err;
use crate::engine::{error::AppError, runtime::vec_codec};
use chain_builder::{Order, QueryBuilder, Sqlite};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Number of embedding rows returned by one keyset-paginated scan.
pub const EMBEDDING_PAGE_SIZE: i64 = 512;

const INDEX_COLS: [&str; 5] = [
    "workspace_id",
    "model_id",
    "dimension",
    "created_at",
    "updated_at",
];

const EMBEDDING_COLS: [&str; 8] = [
    "id",
    "workspace_id",
    "text",
    "embedding",
    "dimension",
    "source_kind",
    "source_id",
    "created_at",
];

const GRAPH_COLS: [&str; 8] = [
    "id",
    "text",
    "embedding",
    "dimension",
    "source_kind",
    "source_id",
    "created_at",
    "updated_at",
];

/// Cap on the number of chunks [`list_for_graph`] returns in one call. The
/// knowledge-graph view needs the whole workspace at once (edges are derived
/// over every pair), so — unlike [`list_embeddings`] — this is not
/// keyset-paginated; see ADR 0007's O(n²) cost note.
pub const GRAPH_NODE_CAP: i64 = 2000;

/// The model identity pinned to one workspace's memory index.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIndexRow {
    pub workspace_id: String,
    pub model_id: String,
    pub dimension: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// A complete stored memory row returned by [`upsert_chunk`].
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MemoryChunkRow {
    pub id: String,
    pub workspace_id: String,
    pub source_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub text: String,
    #[serde(skip)]
    pub embedding: Vec<u8>,
    pub dimension: i64,
    pub content_hash: String,
    pub created_at: String,
    pub updated_at: String,
}

/// One row in the bounded batches consumed by exact-search scoring.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct MemoryEmbeddingRow {
    pub id: String,
    pub workspace_id: String,
    pub text: String,
    pub embedding: Vec<u8>,
    pub dimension: i64,
    pub source_kind: String,
    pub source_id: Option<String>,
    pub created_at: String,
}

/// One row returned by [`list_for_graph`] for the knowledge-graph view.
///
/// Distinct from [`MemoryEmbeddingRow`] because the graph view also needs
/// `updated_at` (node recency), which the exact-search path never reads.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct MemoryGraphRow {
    pub id: String,
    pub text: String,
    pub embedding: Vec<u8>,
    pub dimension: i64,
    pub source_kind: String,
    pub source_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Inputs required to atomically upsert a memory chunk.
pub struct UpsertChunkInput<'a> {
    pub workspace_id: &'a str,
    pub model_id: &'a str,
    pub source_kind: &'a str,
    pub source_id: Option<&'a str>,
    pub text: &'a str,
    pub embedding: &'a [f32],
    pub content_hash: &'a str,
}

/// Result of a chunk upsert. `deduped` is true when the unique
/// `(workspace_id, content_hash)` key already existed and retained its id.
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertChunkResult {
    pub row: MemoryChunkRow,
    pub deduped: bool,
}

/// Fetch the memory-index identity for a workspace.
pub async fn get_index(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Option<MemoryIndexRow>, AppError> {
    QueryBuilder::<Sqlite>::table("memory_index")
        .select(INDEX_COLS)
        .where_eq("workspace_id", workspace_id)
        .fetch_optional::<MemoryIndexRow, _>(pool)
        .await
        .map_err(cb_err)
        .map_err(AppError::from)
}

/// Create a workspace index on first use, or validate its existing identity.
///
/// Raw `sqlx` is required for the idempotent `ON CONFLICT DO NOTHING`. A
/// conflicting row is then read through [`get_index`] and compared explicitly
/// so model or dimension drift becomes [`AppError::Invalid`].
pub async fn ensure_index(
    pool: &SqlitePool,
    workspace_id: &str,
    model_id: &str,
    dimension: usize,
) -> Result<MemoryIndexRow, AppError> {
    validate_model_id(model_id)?;
    let dimension = valid_dimension(dimension)?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO memory_index \
         (workspace_id, model_id, dimension, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?4) \
         ON CONFLICT(workspace_id) DO NOTHING",
    )
    .bind(workspace_id)
    .bind(model_id)
    .bind(dimension)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(AppError::from)?;

    let index = get_index(pool, workspace_id)
        .await?
        .ok_or_else(|| AppError::Internal("memory index disappeared after ensure".into()))?;
    validate_index(&index, model_id, dimension)?;
    Ok(index)
}

/// Normalize, encode, and upsert one memory chunk.
///
/// The index identity is ensured before the write, so every write checks its
/// model and vector dimension. The generated insert id is discarded on a
/// uniqueness conflict; comparing it with the returned stable id reports
/// idempotent deduplication without a racy preflight SELECT.
pub async fn upsert_chunk(
    pool: &SqlitePool,
    input: UpsertChunkInput<'_>,
) -> Result<UpsertChunkResult, AppError> {
    if input.content_hash.trim().is_empty() {
        return Err(AppError::Invalid(
            "memory content hash must not be empty".into(),
        ));
    }

    let normalized = normalize(input.embedding)?;
    let dimension = valid_dimension(normalized.len())?;
    ensure_index(pool, input.workspace_id, input.model_id, normalized.len()).await?;

    let new_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let encoded = vec_codec::encode(&normalized);
    let mut tx = pool.begin().await.map_err(AppError::from)?;

    let row = sqlx::query_as::<_, MemoryChunkRow>(
        "INSERT INTO memory_chunk \
         (id, workspace_id, source_kind, source_id, text, embedding, dimension, \
          content_hash, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9) \
         ON CONFLICT(workspace_id, content_hash) DO UPDATE SET \
            source_kind = excluded.source_kind, \
            source_id = excluded.source_id, \
            text = excluded.text, \
            embedding = excluded.embedding, \
            dimension = excluded.dimension, \
            updated_at = excluded.updated_at \
         RETURNING id, workspace_id, source_kind, source_id, text, embedding, \
                   dimension, content_hash, created_at, updated_at",
    )
    .bind(&new_id)
    .bind(input.workspace_id)
    .bind(input.source_kind)
    .bind(input.source_id)
    .bind(input.text)
    .bind(encoded)
    .bind(dimension)
    .bind(input.content_hash)
    .bind(&now)
    .fetch_one(&mut *tx)
    .await
    .map_err(AppError::from)?;

    sqlx::query("UPDATE memory_index SET updated_at = ?1 WHERE workspace_id = ?2")
        .bind(&now)
        .bind(input.workspace_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::from)?;

    tx.commit().await.map_err(AppError::from)?;

    Ok(UpsertChunkResult {
        deduped: row.id != new_id,
        row,
    })
}

/// Read one bounded page of embeddings for exact search.
///
/// Results use keyset pagination by ascending id: pass the final id from the
/// previous page as `after_id`, or `None` for the first page. Supplying the
/// query embedder identity makes every scan validate model and dimension
/// before returning rows. Every SQL page is scoped by `workspace_id`.
pub async fn list_embeddings(
    pool: &SqlitePool,
    workspace_id: &str,
    model_id: &str,
    dimension: usize,
    after_id: Option<&str>,
) -> Result<Vec<MemoryEmbeddingRow>, AppError> {
    validate_model_id(model_id)?;
    let dimension = valid_dimension(dimension)?;
    let Some(index) = get_index(pool, workspace_id).await? else {
        return Ok(Vec::new());
    };
    validate_index(&index, model_id, dimension)?;

    let mut query = QueryBuilder::<Sqlite>::table("memory_chunk")
        .select(EMBEDDING_COLS)
        .where_eq("workspace_id", workspace_id)
        .order_by("id", Order::Asc)
        .limit(EMBEDDING_PAGE_SIZE);
    if let Some(after_id) = after_id {
        query = query.where_gt("id", after_id);
    }

    let rows = query
        .fetch_all::<MemoryEmbeddingRow, _>(pool)
        .await
        .map_err(cb_err)
        .map_err(AppError::from)?;

    if let Some(row) = rows.iter().find(|row| row.dimension != index.dimension) {
        return Err(AppError::Invalid(format!(
            "stored memory dimension mismatch in workspace {workspace_id}: index is {}, \
             chunk {} is {}",
            index.dimension, row.id, row.dimension
        )));
    }
    Ok(rows)
}

/// Read up to [`GRAPH_NODE_CAP`] chunks for the knowledge-graph view.
///
/// Unlike [`list_embeddings`], this has no model/dimension identity to
/// validate against — the caller derives edges from whatever is stored, and
/// an empty or missing index simply yields an empty list. Returns rows
/// ordered by ascending id so a caller can tell truncation happened by
/// comparing the returned length against [`count`].
pub async fn list_for_graph(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Vec<MemoryGraphRow>, AppError> {
    QueryBuilder::<Sqlite>::table("memory_chunk")
        .select(GRAPH_COLS)
        .where_eq("workspace_id", workspace_id)
        .order_by("id", Order::Asc)
        .limit(GRAPH_NODE_CAP)
        .fetch_all::<MemoryGraphRow, _>(pool)
        .await
        .map_err(cb_err)
        .map_err(AppError::from)
}

/// Delete one chunk from exactly one workspace.
pub async fn delete_chunk(
    pool: &SqlitePool,
    workspace_id: &str,
    id: &str,
) -> Result<bool, AppError> {
    let result = sqlx::query("DELETE FROM memory_chunk WHERE workspace_id = ?1 AND id = ?2")
        .bind(workspace_id)
        .bind(id)
        .execute(pool)
        .await
        .map_err(AppError::from)?;
    Ok(result.rows_affected() > 0)
}

/// Delete every chunk in one workspace, retaining its pinned model identity.
pub async fn clear_workspace(pool: &SqlitePool, workspace_id: &str) -> Result<u64, AppError> {
    let result = sqlx::query("DELETE FROM memory_chunk WHERE workspace_id = ?1")
        .bind(workspace_id)
        .execute(pool)
        .await
        .map_err(AppError::from)?;
    Ok(result.rows_affected())
}

/// Count chunks in one workspace.
pub async fn count(pool: &SqlitePool, workspace_id: &str) -> Result<i64, AppError> {
    QueryBuilder::<Sqlite>::table("memory_chunk")
        .where_eq("workspace_id", workspace_id)
        .count(pool)
        .await
        .map_err(cb_err)
        .map_err(AppError::from)
}

fn validate_model_id(model_id: &str) -> Result<(), AppError> {
    if model_id.trim().is_empty() {
        return Err(AppError::Invalid(
            "memory index model id must not be empty".into(),
        ));
    }
    Ok(())
}

fn valid_dimension(dimension: usize) -> Result<i64, AppError> {
    if dimension == 0 {
        return Err(AppError::Invalid(
            "memory embedding dimension must be greater than zero".into(),
        ));
    }
    i64::try_from(dimension)
        .map_err(|_| AppError::Invalid(format!("memory dimension {dimension} exceeds i64")))
}

fn validate_index(index: &MemoryIndexRow, model_id: &str, dimension: i64) -> Result<(), AppError> {
    if index.model_id != model_id {
        return Err(AppError::Invalid(format!(
            "memory model mismatch for workspace {}: index uses {}, requested {}",
            index.workspace_id, index.model_id, model_id
        )));
    }
    if index.dimension != dimension {
        return Err(AppError::Invalid(format!(
            "memory dimension mismatch for workspace {}: index uses {}, requested {}",
            index.workspace_id, index.dimension, dimension
        )));
    }
    Ok(())
}

fn normalize(vector: &[f32]) -> Result<Vec<f32>, AppError> {
    if vector.is_empty() {
        return Err(AppError::Invalid(
            "memory embedding must not be empty".into(),
        ));
    }

    let mut squared_norm = 0.0f64;
    for value in vector {
        if !value.is_finite() {
            return Err(AppError::Invalid(
                "memory embedding contains a non-finite value".into(),
            ));
        }
        squared_norm += f64::from(*value) * f64::from(*value);
    }
    if squared_norm == 0.0 {
        return Err(AppError::Invalid(
            "memory embedding must have a non-zero norm".into(),
        ));
    }

    let norm = squared_norm.sqrt();
    Ok(vector
        .iter()
        .map(|value| (f64::from(*value) / norm) as f32)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{db::connect_in_memory, repo::workspace};

    const MODEL: &str = "fake-embedder-v1";
    const DIMENSION: usize = 8;

    async fn fixture_workspace(pool: &SqlitePool, name: &str) -> String {
        workspace::create(pool, name, &format!("/tmp/{name}"), None)
            .await
            .expect("create workspace")
            .id
    }

    /// FakeEmbedder-style deterministic pseudo-vector with a seeded hash.
    fn fake_embedding(text: &str, dimension: usize) -> Vec<f32> {
        let mut state = 0xcbf2_9ce4_8422_2325u64;
        for byte in text.bytes() {
            state ^= u64::from(byte);
            state = state.wrapping_mul(0x0000_0100_0000_01b3);
        }

        let mut vector = Vec::with_capacity(dimension);
        for _ in 0..dimension {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let unit = (state as f64 / u64::MAX as f64) * 2.0 - 1.0;
            vector.push(unit as f32);
        }
        normalize(&vector).expect("fake embedding should normalize")
    }

    async fn upsert_fixture(
        pool: &SqlitePool,
        workspace_id: &str,
        text: &str,
        content_hash: &str,
    ) -> UpsertChunkResult {
        let embedding = fake_embedding(text, DIMENSION);
        upsert_chunk(
            pool,
            UpsertChunkInput {
                workspace_id,
                model_id: MODEL,
                source_kind: "manual",
                source_id: None,
                text,
                embedding: &embedding,
                content_hash,
            },
        )
        .await
        .expect("upsert chunk")
    }

    #[tokio::test]
    async fn ensure_index_creates_then_validates_identity() {
        let pool = connect_in_memory().await;
        let workspace_id = fixture_workspace(&pool, "index").await;

        let created = ensure_index(&pool, &workspace_id, MODEL, DIMENSION)
            .await
            .expect("create index");
        let repeated = ensure_index(&pool, &workspace_id, MODEL, DIMENSION)
            .await
            .expect("same identity");
        assert_eq!(created.workspace_id, workspace_id);
        assert_eq!(created.model_id, MODEL);
        assert_eq!(created.dimension, DIMENSION as i64);
        assert_eq!(created.created_at, repeated.created_at);

        assert!(matches!(
            ensure_index(&pool, &workspace_id, "other-model", DIMENSION).await,
            Err(AppError::Invalid(message)) if message.contains("model mismatch")
        ));
        assert!(matches!(
            ensure_index(&pool, &workspace_id, MODEL, DIMENSION + 1).await,
            Err(AppError::Invalid(message)) if message.contains("dimension mismatch")
        ));
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_normalizes_embedding() {
        let pool = connect_in_memory().await;
        let workspace_id = fixture_workspace(&pool, "upsert").await;
        let raw = vec![3.0; DIMENSION];

        let first = upsert_chunk(
            &pool,
            UpsertChunkInput {
                workspace_id: &workspace_id,
                model_id: MODEL,
                source_kind: "agent",
                source_id: Some("agent-1"),
                text: "remember this",
                embedding: &raw,
                content_hash: "same-hash",
            },
        )
        .await
        .expect("first upsert");
        let second = upsert_chunk(
            &pool,
            UpsertChunkInput {
                workspace_id: &workspace_id,
                model_id: MODEL,
                source_kind: "agent",
                source_id: Some("agent-1"),
                text: "remember this",
                embedding: &raw,
                content_hash: "same-hash",
            },
        )
        .await
        .expect("deduped upsert");

        assert!(!first.deduped);
        assert!(second.deduped);
        assert_eq!(first.row.id, second.row.id);
        assert_eq!(count(&pool, &workspace_id).await.unwrap(), 1);

        let decoded = vec_codec::decode(&second.row.embedding, DIMENSION).expect("decode");
        let norm = decoded
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "stored norm was {norm}");
    }

    #[tokio::test]
    async fn writes_and_queries_reject_dimension_or_model_drift() {
        let pool = connect_in_memory().await;
        let workspace_id = fixture_workspace(&pool, "drift").await;
        upsert_fixture(&pool, &workspace_id, "one", "hash-one").await;

        let wrong_dimension = fake_embedding("two", DIMENSION + 1);
        assert!(matches!(
            upsert_chunk(
                &pool,
                UpsertChunkInput {
                    workspace_id: &workspace_id,
                    model_id: MODEL,
                    source_kind: "manual",
                    source_id: None,
                    text: "two",
                    embedding: &wrong_dimension,
                    content_hash: "hash-two",
                },
            )
            .await,
            Err(AppError::Invalid(message)) if message.contains("dimension mismatch")
        ));
        assert!(matches!(
            list_embeddings(&pool, &workspace_id, "other-model", DIMENSION, None).await,
            Err(AppError::Invalid(message)) if message.contains("model mismatch")
        ));
        assert!(matches!(
            list_embeddings(&pool, &workspace_id, MODEL, DIMENSION + 1, None).await,
            Err(AppError::Invalid(message)) if message.contains("dimension mismatch")
        ));

        sqlx::query("UPDATE memory_chunk SET dimension = 7 WHERE workspace_id = ?1")
            .bind(&workspace_id)
            .execute(&pool)
            .await
            .expect("corrupt fixture dimension");
        assert!(matches!(
            list_embeddings(&pool, &workspace_id, MODEL, DIMENSION, None).await,
            Err(AppError::Invalid(message)) if message.contains("stored memory dimension mismatch")
        ));
    }

    #[tokio::test]
    async fn writes_and_queries_reject_invalid_vector_identity() {
        let pool = connect_in_memory().await;
        let workspace_id = fixture_workspace(&pool, "invalid-vector").await;

        for embedding in [&[][..], &[0.0; DIMENSION], &[f32::NAN; DIMENSION]] {
            assert!(matches!(
                upsert_chunk(
                    &pool,
                    UpsertChunkInput {
                        workspace_id: &workspace_id,
                        model_id: MODEL,
                        source_kind: "manual",
                        source_id: None,
                        text: "invalid",
                        embedding,
                        content_hash: "invalid-hash",
                    },
                )
                .await,
                Err(AppError::Invalid(_))
            ));
        }

        assert!(matches!(
            list_embeddings(&pool, &workspace_id, "", DIMENSION, None).await,
            Err(AppError::Invalid(message)) if message.contains("model id")
        ));
        assert!(matches!(
            list_embeddings(&pool, &workspace_id, MODEL, 0, None).await,
            Err(AppError::Invalid(message)) if message.contains("greater than zero")
        ));
    }

    #[tokio::test]
    async fn list_delete_clear_and_count_are_workspace_scoped() {
        let pool = connect_in_memory().await;
        let ws1 = fixture_workspace(&pool, "ws1").await;
        let ws2 = fixture_workspace(&pool, "ws2").await;
        let ws1_chunk = upsert_fixture(&pool, &ws1, "ws1 text", "same-hash").await;
        let ws2_chunk = upsert_fixture(&pool, &ws2, "ws2 text", "same-hash").await;

        let rows = list_embeddings(&pool, &ws1, MODEL, DIMENSION, None)
            .await
            .expect("list ws1");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, ws1_chunk.row.id);
        assert_eq!(rows[0].workspace_id, ws1);

        assert!(!delete_chunk(&pool, &ws1, &ws2_chunk.row.id)
            .await
            .expect("cross-workspace delete"));
        assert_eq!(count(&pool, &ws2).await.unwrap(), 1);
        assert_eq!(clear_workspace(&pool, &ws1).await.unwrap(), 1);
        assert_eq!(count(&pool, &ws1).await.unwrap(), 0);
        assert_eq!(count(&pool, &ws2).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn list_for_graph_returns_updated_at_and_is_workspace_scoped() {
        let pool = connect_in_memory().await;
        let ws1 = fixture_workspace(&pool, "graph-ws1").await;
        let ws2 = fixture_workspace(&pool, "graph-ws2").await;
        let ws1_chunk = upsert_fixture(&pool, &ws1, "ws1 text", "graph-hash").await;
        upsert_fixture(&pool, &ws2, "ws2 text", "graph-hash").await;

        let rows = list_for_graph(&pool, &ws1).await.expect("list for graph");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, ws1_chunk.row.id);
        assert_eq!(rows[0].updated_at, ws1_chunk.row.updated_at);
        assert_eq!(rows[0].created_at, ws1_chunk.row.created_at);
    }

    #[tokio::test]
    async fn list_for_graph_reflects_updates_to_existing_chunks() {
        let pool = connect_in_memory().await;
        let workspace_id = fixture_workspace(&pool, "graph-update").await;
        let first = upsert_fixture(&pool, &workspace_id, "one", "same-hash").await;
        let embedding = fake_embedding("one edited", DIMENSION);
        let second = upsert_chunk(
            &pool,
            UpsertChunkInput {
                workspace_id: &workspace_id,
                model_id: MODEL,
                source_kind: "manual",
                source_id: None,
                text: "one edited",
                embedding: &embedding,
                content_hash: "same-hash",
            },
        )
        .await
        .expect("re-upsert same content hash");

        let rows = list_for_graph(&pool, &workspace_id)
            .await
            .expect("list for graph");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, first.row.id);
        assert_eq!(rows[0].text, "one edited");
        assert_eq!(rows[0].updated_at, second.row.updated_at);
    }

    #[tokio::test]
    async fn list_embeddings_uses_bounded_keyset_pages() {
        let pool = connect_in_memory().await;
        let workspace_id = fixture_workspace(&pool, "pages").await;

        for index in 0..=EMBEDDING_PAGE_SIZE {
            upsert_fixture(
                &pool,
                &workspace_id,
                &format!("memory {index}"),
                &format!("hash-{index}"),
            )
            .await;
        }

        let first = list_embeddings(&pool, &workspace_id, MODEL, DIMENSION, None)
            .await
            .expect("first page");
        assert_eq!(first.len(), EMBEDDING_PAGE_SIZE as usize);
        let after_id = first.last().expect("first page is nonempty").id.clone();
        let second = list_embeddings(&pool, &workspace_id, MODEL, DIMENSION, Some(&after_id))
            .await
            .expect("second page");
        assert_eq!(second.len(), 1);
        assert!(second[0].id > after_id);
    }

    #[tokio::test]
    async fn index_serializes_with_camel_case_fields() {
        let pool = connect_in_memory().await;
        let workspace_id = fixture_workspace(&pool, "serialize").await;
        let index = ensure_index(&pool, &workspace_id, MODEL, DIMENSION)
            .await
            .expect("ensure index");
        let value = serde_json::to_value(index).expect("serialize index");
        assert_eq!(value["workspaceId"], workspace_id);
        assert_eq!(value["modelId"], MODEL);
        assert_eq!(value["dimension"], DIMENSION);
        assert!(value.get("createdAt").is_some());
        assert!(value.get("updatedAt").is_some());
    }
}

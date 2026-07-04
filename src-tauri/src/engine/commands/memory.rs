//! Explicit workspace-memory command handlers and exact vector search.
//!
//! Embedding inference is deliberately separated from persistence/scoring:
//! the production wrappers obtain vectors from the runtime embedder, while
//! [`remember_with_embedding`] and [`search_with_embedding`] provide the
//! deterministic seam used by this lane's tests and benchmarks.

use crate::engine::{
    repo::{
        self,
        memory::{MemoryEmbeddingRow, UpsertChunkInput},
    },
    runtime::{embedder::Embedder, vec_codec},
    AppError, AppState,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, HashMap},
    sync::{Arc, Mutex},
};
use unicode_normalization::UnicodeNormalization;

const DEFAULT_SEARCH_LIMIT: usize = 8;
const MAX_SEARCH_LIMIT: usize = 100;
const MAX_CACHED_WORKSPACES: usize = 4;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RememberReq {
    workspace_id: String,
    text: String,
    source_kind: Option<String>,
    source_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchReq {
    workspace_id: String,
    query: String,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteReq {
    workspace_id: String,
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceReq {
    workspace_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchHit {
    id: String,
    text: String,
    score: f32,
    source_kind: String,
    source_id: Option<String>,
    created_at: String,
}

#[derive(Debug)]
struct CachedMemory {
    id: String,
    text: String,
    vector: Vec<f32>,
    source_kind: String,
    source_id: Option<String>,
    created_at: String,
}

#[derive(Debug)]
struct CachedWorkspace {
    generation: u64,
    model_id: String,
    dimension: usize,
    rows: Arc<Vec<CachedMemory>>,
}

#[derive(Default)]
struct CacheState {
    generations: HashMap<String, u64>,
    entries: HashMap<String, CacheSlot>,
    access_clock: u64,
}

struct CacheSlot {
    entry: Arc<CachedWorkspace>,
    last_access: u64,
}

/// Decoded-vector cache for warm exact search.
///
/// Successful writes invalidate the whole workspace. Cache loads happen
/// outside the mutex and install only if the workspace generation is unchanged,
/// so a concurrent write cannot publish a stale load after invalidation.
/// Searches recheck that generation after scoring and retry if a completed
/// write invalidated the snapshot they used.
///
/// The generation-retry loops are deliberately UNBOUNDED: capping them and
/// returning a stale snapshot would violate the cache's correctness property
/// (never serve results contradicting a completed write). The load-bearing
/// assumption is that memory writes are human/agent-paced — a sustained
/// write storm against one workspace would amplify search latency (repeated
/// reload + rescore), not corrupt results.
#[derive(Default)]
pub struct MemorySearchCache {
    state: Mutex<CacheState>,
}

impl MemorySearchCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn invalidate(&self, workspace_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let generation = state
            .generations
            .entry(workspace_id.to_owned())
            .or_default();
        *generation = generation.wrapping_add(1);
        state.entries.remove(workspace_id);
    }

    fn get(
        &self,
        workspace_id: &str,
        model_id: &str,
        dimension: usize,
    ) -> Option<Arc<CachedWorkspace>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let matches = state.entries.get(workspace_id).is_some_and(|slot| {
            slot.entry.model_id == model_id && slot.entry.dimension == dimension
        });
        if !matches {
            return None;
        }
        state.access_clock = state.access_clock.wrapping_add(1);
        let access = state.access_clock;
        let slot = state
            .entries
            .get_mut(workspace_id)
            .expect("entry existence checked above");
        slot.last_access = access;
        Some(Arc::clone(&slot.entry))
    }

    fn generation(&self, workspace_id: &str) -> u64 {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.generations.get(workspace_id).copied().unwrap_or(0)
    }

    fn install_if_current(
        &self,
        workspace_id: &str,
        expected_generation: u64,
        entry: Arc<CachedWorkspace>,
    ) -> Option<Arc<CachedWorkspace>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let generation = state.generations.get(workspace_id).copied().unwrap_or(0);
        if generation != expected_generation {
            return None;
        }
        state.access_clock = state.access_clock.wrapping_add(1);
        let access = state.access_clock;
        state.entries.insert(
            workspace_id.to_owned(),
            CacheSlot {
                entry: Arc::clone(&entry),
                last_access: access,
            },
        );
        if state.entries.len() > MAX_CACHED_WORKSPACES {
            let evicted = state
                .entries
                .iter()
                .filter(|(candidate, _)| candidate.as_str() != workspace_id)
                .min_by_key(|(_, slot)| slot.last_access)
                .map(|(candidate, _)| candidate.clone());
            if let Some(evicted) = evicted {
                state.entries.remove(&evicted);
            }
        }
        Some(entry)
    }

    #[cfg(test)]
    fn contains(&self, workspace_id: &str) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.entries.contains_key(workspace_id)
    }

    #[cfg(test)]
    fn resident_workspaces(&self) -> usize {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.entries.len()
    }
}

/// Heap entry ordered from worse to better by score, then from larger to
/// smaller id. Wrapping it in [`Reverse`] puts the worst retained hit at the
/// top of the heap, so replacing it keeps memory bounded to `limit`.
#[derive(Debug, Clone)]
struct RankedHit {
    id: String,
    score: f32,
    row_index: usize,
}

impl PartialEq for RankedHit {
    fn eq(&self, other: &Self) -> bool {
        self.score.total_cmp(&other.score) == Ordering::Equal && self.id == other.id
    }
}

impl Eq for RankedHit {}

impl PartialOrd for RankedHit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RankedHit {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.id.cmp(&self.id))
    }
}

async fn require_workspace(state: &AppState, workspace_id: &str) -> Result<(), AppError> {
    if !repo::workspace::exists(&state.db, workspace_id).await? {
        return Err(AppError::NotFound(format!(
            "workspace id={workspace_id} not found"
        )));
    }
    Ok(())
}

fn parse_remember(payload: Value) -> Result<RememberReq, AppError> {
    let req: RememberReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    if req.text.trim().is_empty() {
        return Err(AppError::Invalid("memory text must not be empty".into()));
    }
    Ok(req)
}

fn parse_search(payload: Value) -> Result<SearchReq, AppError> {
    let req: SearchReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    if req.query.trim().is_empty() {
        return Err(AppError::Invalid("memory query must not be empty".into()));
    }
    search_limit(&req)?;
    Ok(req)
}

fn search_limit(req: &SearchReq) -> Result<usize, AppError> {
    let limit = req.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
    if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
        return Err(AppError::Invalid(format!(
            "memory search limit must be between 1 and {MAX_SEARCH_LIMIT}"
        )));
    }
    Ok(limit)
}

async fn validate_source(
    state: &AppState,
    workspace_id: &str,
    source_kind: Option<&str>,
    source_id: Option<&str>,
) -> Result<(&'static str, Option<String>), AppError> {
    match source_kind.unwrap_or("manual") {
        "manual" => {
            if source_id.is_some() {
                return Err(AppError::Invalid(
                    "sourceId is only valid when sourceKind is agent".into(),
                ));
            }
            Ok(("manual", None))
        }
        "agent" => {
            let source_id = source_id.ok_or_else(|| {
                AppError::Invalid("sourceId is required when sourceKind is agent".into())
            })?;
            let agent = repo::workspace_agent::get(&state.db, source_id)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("agent source id={source_id} not found"))
                })?;
            if agent.workspace_id != workspace_id {
                return Err(AppError::Invalid(
                    "agent source does not belong to this workspace".into(),
                ));
            }
            Ok(("agent", Some(source_id.to_owned())))
        }
        other => Err(AppError::Invalid(format!(
            "unknown memory sourceKind: {other}; expected manual or agent"
        ))),
    }
}

fn content_hash(text: &str) -> String {
    let normalized: String = text.nfc().collect();
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}

async fn embed_one(
    embedder: Arc<dyn Embedder>,
    text: String,
) -> Result<(&'static str, Vec<f32>), AppError> {
    let model_id = embedder.model_id();
    let declared_dimension = embedder.dimension();
    let mut vectors = tokio::task::spawn_blocking(move || embedder.embed(&[text]))
        .await
        .map_err(|error| AppError::Internal(format!("embedding worker failed: {error}")))?
        .map_err(|error| AppError::Internal(error.to_string()))?;
    if vectors.len() != 1 {
        return Err(AppError::Internal(format!(
            "embedder contract violation: expected one vector, got {}",
            vectors.len()
        )));
    }
    let vector = vectors.pop().expect("length checked above");
    if vector.len() != declared_dimension {
        return Err(AppError::Internal(format!(
            "embedder contract violation: declared dimension {declared_dimension}, returned {}",
            vector.len()
        )));
    }
    Ok((model_id, vector))
}

fn validate_embedder_identity(
    model_id: &str,
    dimension: usize,
    index: &repo::memory::MemoryIndexRow,
) -> Result<(), AppError> {
    if index.model_id != model_id {
        return Err(AppError::Invalid(format!(
            "memory model mismatch for workspace {}: index uses {}, requested {}",
            index.workspace_id, index.model_id, model_id
        )));
    }
    if usize::try_from(index.dimension).ok() != Some(dimension) {
        return Err(AppError::Invalid(format!(
            "memory dimension mismatch for workspace {}: index uses {}, requested {}",
            index.workspace_id, index.dimension, dimension
        )));
    }
    Ok(())
}

/// `memory.remember` core with an injected runtime embedder.
///
/// Inference runs on `spawn_blocking` and completes before the repository opens
/// its write transaction.
pub async fn remember_with_embedder(
    state: &AppState,
    payload: Value,
    embedder: Arc<dyn Embedder>,
    cache: Arc<MemorySearchCache>,
) -> Result<Value, AppError> {
    let req = parse_remember(payload)?;
    require_workspace(state, &req.workspace_id).await?;
    validate_source(
        state,
        &req.workspace_id,
        req.source_kind.as_deref(),
        req.source_id.as_deref(),
    )
    .await?;
    if let Some(index) = repo::memory::get_index(&state.db, &req.workspace_id).await? {
        validate_embedder_identity(embedder.model_id(), embedder.dimension(), &index)?;
    }

    let (model_id, embedding) = embed_one(Arc::clone(&embedder), req.text.clone()).await?;
    let workspace_id = req.workspace_id.clone();
    let result = remember_with_embedding(state, req, model_id, embedding).await?;
    cache.invalidate(&workspace_id);
    Ok(result)
}

/// `memory.remember` production wrapper.
pub async fn remember(state: &AppState, payload: Value) -> Result<Value, AppError> {
    remember_with_embedder(
        state,
        payload,
        Arc::clone(&state.memory_embedder),
        Arc::clone(&state.memory_search_cache),
    )
    .await
}

/// Persist a precomputed embedding. In production the caller computes the
/// vector through the runtime embedder before this function opens the repo's
/// write transaction.
async fn remember_with_embedding(
    state: &AppState,
    req: RememberReq,
    model_id: &str,
    embedding: Vec<f32>,
) -> Result<Value, AppError> {
    require_workspace(state, &req.workspace_id).await?;
    let (source_kind, source_id) = validate_source(
        state,
        &req.workspace_id,
        req.source_kind.as_deref(),
        req.source_id.as_deref(),
    )
    .await?;
    let hash = content_hash(&req.text);
    let result = repo::memory::upsert_chunk(
        &state.db,
        UpsertChunkInput {
            workspace_id: &req.workspace_id,
            model_id,
            source_kind,
            source_id: source_id.as_deref(),
            text: &req.text,
            embedding: &embedding,
            content_hash: &hash,
        },
    )
    .await?;
    Ok(json!({ "id": result.row.id, "deduped": result.deduped }))
}

fn normalize_query(vector: Vec<f32>, expected_dimension: usize) -> Result<Vec<f32>, AppError> {
    if vector.len() != expected_dimension {
        return Err(AppError::Invalid(format!(
            "memory query dimension mismatch: index uses {expected_dimension}, query has {}",
            vector.len()
        )));
    }

    let mut squared_norm = 0.0f64;
    for value in &vector {
        if !value.is_finite() {
            return Err(AppError::Invalid(
                "memory query embedding contains a non-finite value".into(),
            ));
        }
        squared_norm += f64::from(*value) * f64::from(*value);
    }
    if squared_norm == 0.0 {
        return Err(AppError::Invalid(
            "memory query embedding must have a non-zero norm".into(),
        ));
    }
    let norm = squared_norm.sqrt();
    Ok(vector
        .into_iter()
        .map(|value| (f64::from(value) / norm) as f32)
        .collect())
}

/// Decode one bounded repository page for the warm-search cache.
///
/// Every stored BLOB passes through [`vec_codec::decode`] with the workspace
/// index dimension before its vector can ever reach scoring. A corrupt or
/// wrong-sized BLOB aborts the whole cache load with [`AppError::Invalid`].
fn decode_page(
    rows: Vec<MemoryEmbeddingRow>,
    index_dimension: usize,
) -> Result<Vec<CachedMemory>, AppError> {
    rows.into_iter()
        .map(|row| {
            let vector = vec_codec::decode(&row.embedding, index_dimension)?;
            Ok(CachedMemory {
                id: row.id,
                text: row.text,
                vector,
                source_kind: row.source_kind,
                source_id: row.source_id,
                created_at: row.created_at,
            })
        })
        .collect()
}

async fn load_workspace_cache(
    state: &AppState,
    workspace_id: &str,
    model_id: &str,
    index_dimension: usize,
    cache: &MemorySearchCache,
) -> Result<Arc<CachedWorkspace>, AppError> {
    loop {
        if let Some(entry) = cache.get(workspace_id, model_id, index_dimension) {
            return Ok(entry);
        }
        let generation = cache.generation(workspace_id);
        let mut decoded = Vec::new();
        let mut after_id: Option<String> = None;

        loop {
            let rows = repo::memory::list_embeddings(
                &state.db,
                workspace_id,
                model_id,
                index_dimension,
                after_id.as_deref(),
            )
            .await?;
            if rows.is_empty() {
                break;
            }

            let page_len = rows.len();
            after_id = rows.last().map(|row| row.id.clone());
            let page = tokio::task::spawn_blocking(move || decode_page(rows, index_dimension))
                .await
                .map_err(|error| {
                    AppError::Internal(format!("memory cache worker failed: {error}"))
                })??;
            decoded.extend(page);

            if page_len < repo::memory::EMBEDDING_PAGE_SIZE as usize {
                break;
            }
        }

        let entry = Arc::new(CachedWorkspace {
            generation,
            model_id: model_id.to_owned(),
            dimension: index_dimension,
            rows: Arc::new(decoded),
        });
        if let Some(installed) =
            cache.install_if_current(workspace_id, generation, Arc::clone(&entry))
        {
            return Ok(installed);
        }
    }
}

fn score_cached(
    query: &[f32],
    rows: &[CachedMemory],
    limit: usize,
) -> Result<Vec<SearchHit>, AppError> {
    let mut heap = BinaryHeap::<Reverse<RankedHit>>::with_capacity(limit + 1);
    for (row_index, row) in rows.iter().enumerate() {
        let score = query
            .iter()
            .zip(row.vector.iter())
            .map(|(left, right)| f64::from(*left) * f64::from(*right))
            .sum::<f64>() as f32;
        if !score.is_finite() {
            return Err(AppError::Invalid(format!(
                "memory score for chunk {} is non-finite",
                row.id
            )));
        }

        let should_insert = heap.len() < limit
            || heap.peek().is_some_and(|worst| {
                score.total_cmp(&worst.0.score) == Ordering::Greater
                    || (score.total_cmp(&worst.0.score) == Ordering::Equal && row.id < worst.0.id)
            });
        if should_insert {
            let candidate = RankedHit {
                id: row.id.clone(),
                score,
                row_index,
            };
            if heap.len() == limit {
                heap.pop();
            }
            heap.push(Reverse(candidate));
        }
    }

    let mut ranked: Vec<RankedHit> = heap.into_iter().map(|Reverse(ranked)| ranked).collect();
    ranked.sort_unstable_by(|left, right| right.cmp(left));
    Ok(ranked
        .into_iter()
        .map(|ranked| {
            let row = &rows[ranked.row_index];
            SearchHit {
                id: row.id.clone(),
                text: row.text.clone(),
                score: ranked.score,
                source_kind: row.source_kind.clone(),
                source_id: row.source_id.clone(),
                created_at: row.created_at.clone(),
            }
        })
        .collect())
}

/// Exact top-k search for a precomputed query embedding.
///
/// Repository reads remain async and workspace-scoped. Each bounded page is
/// moved to `spawn_blocking` for BLOB decode, and cached dot-product scoring
/// also runs there, keeping CPU-heavy work off Tokio executor threads.
async fn search_with_embedding(
    state: &AppState,
    req: SearchReq,
    model_id: &str,
    query_embedding: Vec<f32>,
    cache: Arc<MemorySearchCache>,
) -> Result<Value, AppError> {
    require_workspace(state, &req.workspace_id).await?;
    let limit = search_limit(&req)?;
    let Some(index) = repo::memory::get_index(&state.db, &req.workspace_id).await? else {
        return Ok(json!({ "hits": [] }));
    };
    let index_dimension = usize::try_from(index.dimension).map_err(|_| {
        AppError::Invalid(format!(
            "memory index dimension {} is invalid",
            index.dimension
        ))
    })?;
    let query = normalize_query(query_embedding, index_dimension)?;
    loop {
        let cached =
            load_workspace_cache(state, &req.workspace_id, model_id, index_dimension, &cache)
                .await?;
        let generation = cached.generation;
        let query = query.clone();
        let hits = tokio::task::spawn_blocking(move || score_cached(&query, &cached.rows, limit))
            .await
            .map_err(|error| {
                AppError::Internal(format!("memory search worker failed: {error}"))
            })??;

        if cache.generation(&req.workspace_id) == generation {
            return Ok(json!({ "hits": hits }));
        }
    }
}

/// `memory.search` core with an injected runtime embedder.
///
/// Empty indexes return no hits without initializing the model. Otherwise the
/// index identity is checked before inference, then both inference and scoring
/// run on blocking workers.
pub async fn search_with_embedder(
    state: &AppState,
    payload: Value,
    embedder: Arc<dyn Embedder>,
    cache: Arc<MemorySearchCache>,
) -> Result<Value, AppError> {
    let req = parse_search(payload)?;
    require_workspace(state, &req.workspace_id).await?;
    let Some(index) = repo::memory::get_index(&state.db, &req.workspace_id).await? else {
        return Ok(json!({ "hits": [] }));
    };
    validate_embedder_identity(embedder.model_id(), embedder.dimension(), &index)?;

    let (model_id, embedding) = embed_one(Arc::clone(&embedder), req.query.clone()).await?;
    search_with_embedding(state, req, model_id, embedding, cache).await
}

/// `memory.search` production wrapper.
pub async fn search(state: &AppState, payload: Value) -> Result<Value, AppError> {
    search_with_embedder(
        state,
        payload,
        Arc::clone(&state.memory_embedder),
        Arc::clone(&state.memory_search_cache),
    )
    .await
}

/// `memory.delete` — delete one workspace-scoped chunk idempotently.
pub async fn delete_with_cache(
    state: &AppState,
    payload: Value,
    cache: Arc<MemorySearchCache>,
) -> Result<Value, AppError> {
    let req: DeleteReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    let deleted = repo::memory::delete_chunk(&state.db, &req.workspace_id, &req.id).await?;
    if deleted {
        cache.invalidate(&req.workspace_id);
    }
    Ok(json!({ "deleted": deleted }))
}

/// `memory.delete` production wrapper.
pub async fn delete(state: &AppState, payload: Value) -> Result<Value, AppError> {
    delete_with_cache(state, payload, Arc::clone(&state.memory_search_cache)).await
}

/// `memory.clear` — delete all chunks in one workspace.
pub async fn clear_with_cache(
    state: &AppState,
    payload: Value,
    cache: Arc<MemorySearchCache>,
) -> Result<Value, AppError> {
    let req: WorkspaceReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    let deleted = repo::memory::clear_workspace(&state.db, &req.workspace_id).await?;
    cache.invalidate(&req.workspace_id);
    Ok(json!({ "deleted": deleted }))
}

/// `memory.clear` production wrapper.
pub async fn clear(state: &AppState, payload: Value) -> Result<Value, AppError> {
    clear_with_cache(state, payload, Arc::clone(&state.memory_search_cache)).await
}

/// `memory.status` core with an injected runtime embedder.
///
/// Readiness comes directly from [`Embedder::is_ready`], which is cheap and
/// side-effect free; status never initializes or downloads a model.
pub async fn status_with_embedder(
    state: &AppState,
    payload: Value,
    embedder: Arc<dyn Embedder>,
) -> Result<Value, AppError> {
    let req: WorkspaceReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    let chunks = repo::memory::count(&state.db, &req.workspace_id).await?;
    let index = repo::memory::get_index(&state.db, &req.workspace_id).await?;
    if let Some(index) = index.as_ref() {
        validate_embedder_identity(embedder.model_id(), embedder.dimension(), index)?;
    }

    let mut result = serde_json::Map::new();
    result.insert("chunks".into(), json!(chunks));
    result.insert("modelReady".into(), json!(embedder.is_ready()));
    if let Some(index) = index {
        result.insert("modelId".into(), json!(index.model_id));
        result.insert("dimension".into(), json!(index.dimension));
    }
    Ok(Value::Object(result))
}

/// `memory.status` production wrapper.
pub async fn status(state: &AppState, payload: Value) -> Result<Value, AppError> {
    status_with_embedder(state, payload, Arc::clone(&state.memory_embedder)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        repo::{
            agent_definition::{self, AgentDefinitionInput},
            memory, workspace, workspace_agent,
        },
        router,
        runtime::{
            embedder::{EmbedError, Embedder, FakeEmbedder},
            vec_codec,
        },
    };
    use sqlx::SqlitePool;
    use std::time::{Duration, Instant};

    const MODEL: &str = "fake-embedder";
    const DIMENSION: usize = 8;

    struct NotReadyEmbedder(FakeEmbedder);

    impl Embedder for NotReadyEmbedder {
        fn model_id(&self) -> &'static str {
            self.0.model_id()
        }

        fn dimension(&self) -> usize {
            self.0.dimension()
        }

        fn is_ready(&self) -> bool {
            false
        }

        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            self.0.embed(texts)
        }
    }

    async fn fixture_workspace(state: &AppState, name: &str) -> String {
        workspace::create(&state.db, name, &format!("/tmp/{name}"), None)
            .await
            .expect("create workspace")
            .id
    }

    fn fake_embedding(text: &str, dimension: usize) -> Vec<f32> {
        FakeEmbedder::new(dimension)
            .embed(&[text.to_owned()])
            .expect("fake embedder")[0]
            .clone()
    }

    async fn remember_text(
        state: &AppState,
        cache: Arc<MemorySearchCache>,
        workspace_id: &str,
        text: &str,
    ) -> Result<Value, AppError> {
        remember_with_embedder(
            state,
            json!({ "workspaceId": workspace_id, "text": text }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            cache,
        )
        .await
    }

    #[tokio::test]
    async fn remember_is_idempotent_for_nfc_equivalent_text() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let workspace_id = fixture_workspace(&state, "remember").await;
        let composed = "café";
        let decomposed = "cafe\u{301}";
        assert_eq!(content_hash(composed), content_hash(decomposed));

        let first = remember_text(&state, Arc::clone(&cache), &workspace_id, composed)
            .await
            .expect("first remember");
        let second = remember_text(&state, Arc::clone(&cache), &workspace_id, decomposed)
            .await
            .expect("deduped remember");
        assert_eq!(first["id"], second["id"]);
        assert_eq!(first["deduped"], false);
        assert_eq!(second["deduped"], true);
    }

    #[tokio::test]
    async fn remember_validates_agent_source_workspace() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws1 = fixture_workspace(&state, "source-ws1").await;
        let ws2 = fixture_workspace(&state, "source-ws2").await;
        let definition = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: "Source".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create definition");
        let source = workspace_agent::instantiate(&state.db, &ws2, &definition.id)
            .await
            .expect("instantiate source");

        let error = remember_with_embedder(
            &state,
            json!({
                "workspaceId": ws1,
                "text": "cross workspace",
                "sourceKind": "agent",
                "sourceId": source.id,
            }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            cache,
        )
        .await
        .expect_err("cross-workspace source must fail");
        assert!(matches!(error, AppError::Invalid(message) if message.contains("does not belong")));
    }

    #[tokio::test]
    async fn search_returns_exact_top_k_and_never_crosses_workspaces() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws1 = fixture_workspace(&state, "search-ws1").await;
        let ws2 = fixture_workspace(&state, "search-ws2").await;
        let mut ws1_ids = Vec::new();
        for text in ["alpha", "beta", "gamma", "delta"] {
            let remembered = remember_text(&state, Arc::clone(&cache), &ws1, text)
                .await
                .expect("remember ws1");
            ws1_ids.push(remembered["id"].as_str().unwrap().to_owned());
        }
        remember_text(&state, Arc::clone(&cache), &ws2, "alpha")
            .await
            .expect("remember ws2");

        let result = search_with_embedder(
            &state,
            json!({ "workspaceId": ws1, "query": "alpha", "limit": 2 }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect("search");
        let hits = result["hits"].as_array().expect("hits array");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0]["text"], "alpha");
        assert!(
            hits.iter().all(|hit| hit.get("sourceId").is_some()),
            "fixed search result shape includes sourceId, including null for manual memories"
        );
        assert!(hits[0]["score"].as_f64().unwrap() >= hits[1]["score"].as_f64().unwrap());
        assert!(hits.iter().all(|hit| {
            ws1_ids
                .iter()
                .any(|id| Some(id.as_str()) == hit["id"].as_str())
        }));
    }

    #[tokio::test]
    async fn search_decodes_every_blob_and_rejects_corruption() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let workspace_id = fixture_workspace(&state, "corrupt").await;
        remember_text(&state, Arc::clone(&cache), &workspace_id, "valid")
            .await
            .expect("remember");
        sqlx::query("UPDATE memory_chunk SET embedding = x'000102' WHERE workspace_id = ?1")
            .bind(&workspace_id)
            .execute(&state.db)
            .await
            .expect("corrupt fixture");

        let error = search_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id, "query": "valid" }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            cache,
        )
        .await
        .expect_err("corrupt BLOB must fail");
        assert!(
            matches!(error, AppError::Invalid(message) if message.contains("BLOB length mismatch"))
        );
    }

    #[tokio::test]
    async fn delete_clear_and_status_match_fixed_wire_shapes() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let workspace_id = fixture_workspace(&state, "lifecycle").await;
        let remembered = remember_text(&state, Arc::clone(&cache), &workspace_id, "one")
            .await
            .expect("remember");
        remember_text(&state, Arc::clone(&cache), &workspace_id, "two")
            .await
            .expect("remember second");

        let status = status_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
        )
        .await
        .expect("status");
        assert_eq!(status["chunks"], 2);
        assert_eq!(status["modelId"], MODEL);
        assert_eq!(status["dimension"], DIMENSION);
        assert_eq!(status["modelReady"], true);

        search_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id, "query": "one" }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect("warm cache");
        assert!(cache.contains(&workspace_id));

        let deleted = delete_with_cache(
            &state,
            json!({ "workspaceId": workspace_id, "id": remembered["id"] }),
            Arc::clone(&cache),
        )
        .await
        .expect("delete");
        assert_eq!(deleted, json!({ "deleted": true }));

        assert!(!cache.contains(&workspace_id), "delete must invalidate");

        let cleared = clear_with_cache(
            &state,
            json!({ "workspaceId": workspace_id }),
            Arc::clone(&cache),
        )
        .await
        .expect("clear");
        assert_eq!(cleared, json!({ "deleted": 1 }));
        assert!(!cache.contains(&workspace_id), "clear must invalidate");
    }

    #[tokio::test]
    async fn status_sources_model_readiness_from_the_embedder() {
        let state = AppState::for_tests().await;
        let workspace_id = fixture_workspace(&state, "not-ready").await;

        let status = status_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id }),
            Arc::new(NotReadyEmbedder(FakeEmbedder::new(DIMENSION))),
        )
        .await
        .expect("status");

        assert_eq!(status["chunks"], 0);
        assert_eq!(status["modelReady"], false);
        assert!(status.get("modelId").is_none());
        assert!(status.get("dimension").is_none());
    }

    #[tokio::test]
    async fn completed_remember_invalidates_warm_cache() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let workspace_id = fixture_workspace(&state, "remember-invalidation").await;
        remember_text(&state, Arc::clone(&cache), &workspace_id, "one")
            .await
            .expect("remember one");
        search_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id, "query": "one" }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect("warm cache");
        assert!(cache.contains(&workspace_id));

        remember_text(&state, Arc::clone(&cache), &workspace_id, "two")
            .await
            .expect("remember two");
        assert!(
            !cache.contains(&workspace_id),
            "completed remember must invalidate"
        );
        let result = search_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id, "query": "two" }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect("rebuilt search");
        assert_eq!(result["hits"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn search_cache_is_lru_bounded_to_four_workspaces() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let mut workspaces = Vec::new();

        for index in 0..5 {
            let workspace_id = fixture_workspace(&state, &format!("lru-{index}")).await;
            remember_text(
                &state,
                Arc::clone(&cache),
                &workspace_id,
                &format!("memory {index}"),
            )
            .await
            .expect("remember fixture");
            search_with_embedder(
                &state,
                json!({ "workspaceId": workspace_id, "query": format!("memory {index}") }),
                Arc::new(FakeEmbedder::new(DIMENSION)),
                Arc::clone(&cache),
            )
            .await
            .expect("populate cache");
            workspaces.push(workspace_id);

            if index == 3 {
                search_with_embedder(
                    &state,
                    json!({ "workspaceId": workspaces[0], "query": "memory 0" }),
                    Arc::new(FakeEmbedder::new(DIMENSION)),
                    Arc::clone(&cache),
                )
                .await
                .expect("touch oldest workspace");
            }
        }

        assert_eq!(cache.resident_workspaces(), MAX_CACHED_WORKSPACES);
        assert!(
            cache.contains(&workspaces[0]),
            "recently touched entry stays"
        );
        assert!(
            !cache.contains(&workspaces[1]),
            "least-recent entry evicted"
        );
        assert!(cache.contains(&workspaces[4]), "new entry is resident");
    }

    #[test]
    fn invalidation_rejects_an_inflight_stale_cache_install() {
        let cache = MemorySearchCache::new();
        let generation = cache.generation("ws");
        cache.invalidate("ws");
        let stale = Arc::new(CachedWorkspace {
            generation,
            model_id: MODEL.into(),
            dimension: DIMENSION,
            rows: Arc::new(Vec::new()),
        });

        assert!(
            cache.install_if_current("ws", generation, stale).is_none(),
            "a load begun before invalidation must never become resident"
        );
        assert_eq!(cache.resident_workspaces(), 0);
    }

    #[tokio::test]
    async fn router_dispatches_all_fixed_memory_commands() {
        let state = AppState::for_tests().await;
        let workspace_id = fixture_workspace(&state, "router").await;

        let first = router::dispatch(
            &state,
            "memory.remember",
            json!({ "workspaceId": workspace_id, "text": "router memory" }),
        )
        .await
        .expect("remember route");
        assert_eq!(first["deduped"], false);
        let repeated = router::dispatch(
            &state,
            "memory.remember",
            json!({ "workspaceId": workspace_id, "text": "router memory" }),
        )
        .await
        .expect("repeat remember route");
        assert_eq!(repeated["id"], first["id"]);
        assert_eq!(repeated["deduped"], true);

        let search = router::dispatch(
            &state,
            "memory.search",
            json!({ "workspaceId": workspace_id, "query": "router memory" }),
        )
        .await
        .expect("search route");
        assert_eq!(search["hits"].as_array().unwrap().len(), 1);

        let status = router::dispatch(
            &state,
            "memory.status",
            json!({ "workspaceId": workspace_id }),
        )
        .await
        .expect("status route");
        assert_eq!(status["chunks"], 1);
        assert_eq!(status["modelReady"], true);

        let deleted = router::dispatch(
            &state,
            "memory.delete",
            json!({ "workspaceId": workspace_id, "id": first["id"] }),
        )
        .await
        .expect("delete route");
        assert_eq!(deleted, json!({ "deleted": true }));

        let cleared = router::dispatch(
            &state,
            "memory.clear",
            json!({ "workspaceId": workspace_id }),
        )
        .await
        .expect("clear route");
        assert_eq!(cleared, json!({ "deleted": 0 }));
    }

    #[test]
    fn bounded_heap_matches_full_sort_reference() {
        let query = fake_embedding("query", DIMENSION);
        let rows: Vec<CachedMemory> = (0..50)
            .map(|index| {
                let vector = fake_embedding(&format!("row-{index}"), DIMENSION);
                CachedMemory {
                    id: format!("{index:03}"),
                    text: format!("row-{index}"),
                    vector,
                    source_kind: "manual".into(),
                    source_id: None,
                    created_at: "2026-07-04T00:00:00Z".into(),
                }
            })
            .collect();
        let mut expected: Vec<(String, f32)> = rows
            .iter()
            .map(|row| {
                let score = query
                    .iter()
                    .zip(&row.vector)
                    .map(|(left, right)| f64::from(*left) * f64::from(*right))
                    .sum::<f64>() as f32;
                (row.id.clone(), score)
            })
            .collect();
        expected.sort_unstable_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });

        let actual: Vec<(String, f32)> = score_cached(&query, &rows, 7)
            .expect("score")
            .into_iter()
            .map(|hit| (hit.id, hit.score))
            .collect();
        assert_eq!(actual, expected[..7]);
    }

    async fn insert_benchmark_fixture(
        pool: &SqlitePool,
        workspace_id: &str,
        rows: usize,
        dimension: usize,
    ) {
        memory::ensure_index(pool, workspace_id, MODEL, dimension)
            .await
            .expect("ensure benchmark index");
        let mut tx = pool.begin().await.expect("begin fixture transaction");
        for index in 0..rows {
            let vector = fake_embedding(&format!("memory {index}"), dimension);
            let bytes = vec_codec::encode(&vector);
            sqlx::query(
                "INSERT INTO memory_chunk \
                 (id, workspace_id, source_kind, text, embedding, dimension, content_hash, \
                  created_at, updated_at) \
                 VALUES (?1, ?2, 'manual', ?3, ?4, ?5, ?6, \
                         '2026-07-04T00:00:00Z', '2026-07-04T00:00:00Z')",
            )
            .bind(format!("{workspace_id}-chunk-{index:08}"))
            .bind(workspace_id)
            .bind(format!("memory {index}"))
            .bind(bytes)
            .bind(dimension as i64)
            .bind(format!("hash-{index}"))
            .execute(&mut *tx)
            .await
            .expect("insert benchmark row");
        }
        tx.commit().await.expect("commit fixture transaction");
    }

    fn percentile_95(samples: &mut [Duration]) -> Duration {
        samples.sort_unstable();
        let index = ((samples.len() as f64 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(samples.len() - 1);
        samples[index]
    }

    async fn benchmark_workspace(
        state: &AppState,
        cache: Arc<MemorySearchCache>,
        rows: usize,
    ) -> (Duration, Duration) {
        let workspace_id = fixture_workspace(state, &format!("benchmark-{rows}")).await;
        insert_benchmark_fixture(&state.db, &workspace_id, rows, 384).await;
        let query = fake_embedding("benchmark query", 384);
        let request = || SearchReq {
            workspace_id: workspace_id.clone(),
            query: "benchmark query".into(),
            limit: Some(8),
        };

        let cold_started = Instant::now();
        search_with_embedding(state, request(), MODEL, query.clone(), Arc::clone(&cache))
            .await
            .expect("cold cache build");
        let cold = cold_started.elapsed();
        let mut samples = Vec::with_capacity(20);
        for _ in 0..20 {
            let started = Instant::now();
            let result =
                search_with_embedding(state, request(), MODEL, query.clone(), Arc::clone(&cache))
                    .await
                    .expect("benchmark search");
            assert_eq!(result["hits"].as_array().unwrap().len(), 8);
            samples.push(started.elapsed());
        }
        (cold, percentile_95(&mut samples))
    }

    /// Run with:
    /// `cargo test --release benchmark_exact_search_10k_50k -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "release-only 10k/50k exact-search performance gate"]
    async fn benchmark_exact_search_10k_50k() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let (cold_10k, p95_10k) = benchmark_workspace(&state, Arc::clone(&cache), 10_000).await;
        let (cold_50k, p95_50k) = benchmark_workspace(&state, Arc::clone(&cache), 50_000).await;
        println!(
            "memory exact search: cold 10k={:.3}ms 50k={:.3}ms; \
             warm p95 10k={:.3}ms 50k={:.3}ms (20 samples each)",
            cold_10k.as_secs_f64() * 1000.0,
            cold_50k.as_secs_f64() * 1000.0,
            p95_10k.as_secs_f64() * 1000.0,
            p95_50k.as_secs_f64() * 1000.0
        );
        assert!(
            p95_50k < Duration::from_millis(100),
            "50k warm p95 {:.3}ms exceeds 100ms gate",
            p95_50k.as_secs_f64() * 1000.0
        );
    }

    #[test]
    fn request_validation_enforces_fixed_defaults_and_bounds() {
        let req = parse_search(json!({ "workspaceId": "ws", "query": "q" })).unwrap();
        assert_eq!(search_limit(&req).unwrap(), DEFAULT_SEARCH_LIMIT);
        assert!(parse_search(json!({ "workspaceId": "ws", "query": "", "limit": 8 })).is_err());
        assert!(parse_search(json!({ "workspaceId": "ws", "query": "q", "limit": 0 })).is_err());
        assert!(parse_search(json!({ "workspaceId": "ws", "query": "q", "limit": 101 })).is_err());
        assert!(parse_remember(json!({ "workspaceId": "ws", "text": " " })).is_err());
    }
}

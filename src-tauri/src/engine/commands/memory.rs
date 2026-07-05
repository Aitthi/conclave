//! Explicit workspace-memory command handlers and hybrid vector+keyword search.
//!
//! Retrieval is two-stage ([`score_cached`]): exact brute-force cosine pulls
//! the top candidates, then each is re-scored as `0.6·cosine + 0.4·bm25_norm`
//! (Okapi BM25 over the candidate set only) and the top `limit` returned. Pure
//! cosine underweights exact tokens — UUIDs, error codes, crate names — which
//! are the shape of our memories; the keyword term lifts those. The fused
//! `score` stays a single f32, so the result shape is unchanged.
//!
//! Embedding inference is deliberately separated from persistence/scoring:
//! the production wrappers obtain vectors from the runtime embedder, while
//! [`remember_with_embedding`] and [`search_with_embedding`] provide the
//! deterministic seam used by this module's tests and the retrieval bench
//! ([`super::memory_bench`]).

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
    collections::{BinaryHeap, HashMap, HashSet},
    sync::{Arc, Mutex},
};
use unicode_normalization::UnicodeNormalization;

const DEFAULT_SEARCH_LIMIT: usize = 8;
const MAX_SEARCH_LIMIT: usize = 100;
const MAX_CACHED_WORKSPACES: usize = 4;

/// `memory.graph` `related`-edge threshold (ADR 0007). Tuned against the real
/// workspace `11ecf99b-53f4-4c24-b538-b19e5933a9e3` store (11 chunks, 55
/// pairs): 0.45 yields 7 edges with max node degree 2 — neither fully
/// connected nor edgeless, and well under [`RELATED_TOP_K`] per node.
const RELATED_SIMILARITY_THRESHOLD: f32 = 0.45;
/// Per-node cap on `related` edges before threshold filtering.
const RELATED_TOP_K: usize = 3;

/// Hybrid-rank fusion weights (task `memory-hybrid-bench`). Pure cosine
/// underweights exact tokens — proper nouns, UUIDs, error codes, crate names —
/// which is the shape of our memories, so search re-ranks the cosine
/// candidates with a keyword term. `0.6/0.4` are MemPalace's defaults, adopted
/// as a starting point and validated by the retrieval bench
/// (`commands/memory_bench.rs`); they are constants, not config surface.
const COSINE_WEIGHT: f32 = 0.6;
const BM25_WEIGHT: f32 = 0.4;
/// Okapi BM25 term-frequency saturation (`k1`) and length-normalization (`b`).
const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;
/// Stage-1 candidate floor: pull at least this many cosine neighbours (or
/// `4·limit` when larger) into the keyword re-rank, capped at the corpus size.
const HYBRID_CANDIDATE_FLOOR: usize = 50;

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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProposeReq {
    workspace_id: String,
    proposer_id: String,
    text: String,
    source_note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueReq {
    workspace_id: String,
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewReq {
    workspace_id: String,
    reviewer_id: String,
    proposal_id: String,
    reason: Option<String>,
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

/// Emit `memory:changed` after a workspace's `memory_chunk` table actually
/// changed — mirrors `commands::task::emit_changed` (non-fatal; a UI refresh
/// miss is not a request failure, same as every other `bus::*` emit call).
///
/// Callers MUST gate this on an actual write: never call it for a no-op (a
/// deduped `remember`/`approve`, a `delete` that found nothing, an empty
/// `clear`) — the graph only needs to refetch when there's something new,
/// and emitting on origin (UDS CLI vs UI) must never differ (risk ledger).
fn emit_changed(state: &AppState, workspace_id: &str) {
    #[cfg(test)]
    emit_probe().lock().unwrap().push(workspace_id.to_string());
    state.emit(
        crate::engine::bus::MEMORY_CHANGED,
        crate::engine::bus::MemoryChanged {
            workspace_id: workspace_id.to_string(),
        },
    );
}

/// Test-only probe: records every workspace id [`emit_changed`] was called
/// with, so tests can assert "emitted on success, not on a no-op" without an
/// `AppHandle` — `AppState::for_tests` intentionally omits one (see
/// `state.rs`, out of this task's boundary), so `state.emit` alone is
/// unobservable from here. Safe under parallel test execution: each test
/// uses its own freshly-created (unique) `fixture_workspace` id and only
/// ever filters this global log for entries matching ITS id.
#[cfg(test)]
fn emit_probe() -> &'static Mutex<Vec<String>> {
    static PROBE: std::sync::OnceLock<Mutex<Vec<String>>> = std::sync::OnceLock::new();
    PROBE.get_or_init(|| Mutex::new(Vec::new()))
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
    if !result.deduped {
        emit_changed(state, &req.workspace_id);
    }
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

/// Lowercase, split on non-alphanumeric boundaries, drop empties. Deliberately
/// dumb and deterministic — no stemming, no stopword list — so `SqlSafeStr`,
/// `RFC3339`, and `q8` survive as single matchable tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Stage 1 of the hybrid rank: exact brute-force cosine over every row, keeping
/// the `target` best as re-rank candidates. Bounded heap holds memory to
/// `target`; ties break on chunk id so the candidate set is deterministic. A
/// non-finite score is a hard error, as it was for the pre-hybrid path. Returns
/// candidates already sorted best-cosine-first.
fn cosine_candidates(
    query: &[f32],
    rows: &[CachedMemory],
    target: usize,
) -> Result<Vec<RankedHit>, AppError> {
    let mut heap = BinaryHeap::<Reverse<RankedHit>>::with_capacity(target + 1);
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

        let should_insert = heap.len() < target
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
            if heap.len() == target {
                heap.pop();
            }
            heap.push(Reverse(candidate));
        }
    }

    let mut ranked: Vec<RankedHit> = heap.into_iter().map(|Reverse(ranked)| ranked).collect();
    ranked.sort_unstable_by(|left, right| right.cmp(left));
    Ok(ranked)
}

/// Okapi BM25 relevance of one candidate document to the query terms, computed
/// over the candidate set only (no global keyword index). `document_frequency`
/// counts how many candidates contain each term; `average_length` and
/// `candidate_count` describe the candidate set. Returns the raw (un-normalized)
/// score.
fn bm25_score(
    document: &[String],
    query_terms: &[String],
    document_frequency: &HashMap<&str, usize>,
    average_length: f32,
    candidate_count: f32,
) -> f32 {
    if document.is_empty() || query_terms.is_empty() {
        return 0.0;
    }
    let length = document.len() as f32;
    let average_length = if average_length > 0.0 { average_length } else { 1.0 };
    query_terms
        .iter()
        .map(|term| {
            let term_frequency = document
                .iter()
                .filter(|token| token.as_str() == term.as_str())
                .count() as f32;
            if term_frequency == 0.0 {
                return 0.0;
            }
            let df = *document_frequency.get(term.as_str()).unwrap_or(&0) as f32;
            // BM25+ IDF: the `+1` inside the log keeps it non-negative even when
            // a term appears in more than half the candidate set.
            let idf = ((candidate_count - df + 0.5) / (df + 0.5) + 1.0).ln();
            let denominator =
                term_frequency + BM25_K1 * (1.0 - BM25_B + BM25_B * length / average_length);
            idf * (term_frequency * (BM25_K1 + 1.0)) / denominator
        })
        .sum()
}

/// Two-stage hybrid rank. Stage 1 pulls the top cosine candidates; stage 2
/// re-scores each as `0.6·cosine + 0.4·bm25_norm` (BM25 normalized to `[0,1]`
/// by the max over the candidate set) and returns the top `limit`. Ties break
/// on chunk id, so the same store and query always yield the same ranking. The
/// returned `score` is the single fused value — the search result shape is
/// unchanged from the pre-hybrid pure-cosine path.
fn score_cached(
    query: &[f32],
    query_text: &str,
    rows: &[CachedMemory],
    limit: usize,
) -> Result<Vec<SearchHit>, AppError> {
    let target = (4 * limit).max(HYBRID_CANDIDATE_FLOOR).min(rows.len());
    let candidates = cosine_candidates(query, rows, target)?;

    let query_terms: Vec<String> = {
        let mut terms = tokenize(query_text);
        terms.sort();
        terms.dedup();
        terms
    };
    let documents: Vec<Vec<String>> = candidates
        .iter()
        .map(|candidate| tokenize(&rows[candidate.row_index].text))
        .collect();
    let candidate_count = candidates.len() as f32;
    let average_length = if candidates.is_empty() {
        0.0
    } else {
        documents.iter().map(Vec::len).sum::<usize>() as f32 / candidate_count
    };
    let mut document_frequency: HashMap<&str, usize> = HashMap::new();
    for term in &query_terms {
        let occurrences = documents
            .iter()
            .filter(|document| document.iter().any(|token| token == term))
            .count();
        if occurrences > 0 {
            document_frequency.insert(term.as_str(), occurrences);
        }
    }

    let raw_bm25: Vec<f32> = documents
        .iter()
        .map(|document| {
            bm25_score(
                document,
                &query_terms,
                &document_frequency,
                average_length,
                candidate_count,
            )
        })
        .collect();
    let max_bm25 = raw_bm25.iter().copied().fold(0.0f32, f32::max);

    let mut fused: Vec<(usize, String, f32)> = candidates
        .iter()
        .enumerate()
        .map(|(candidate_index, candidate)| {
            let bm25_norm = if max_bm25 > 0.0 {
                raw_bm25[candidate_index] / max_bm25
            } else {
                0.0
            };
            let score = COSINE_WEIGHT * candidate.score + BM25_WEIGHT * bm25_norm;
            (candidate.row_index, candidate.id.clone(), score)
        })
        .collect();
    fused.sort_by(|left, right| {
        right
            .2
            .total_cmp(&left.2)
            .then_with(|| left.1.cmp(&right.1))
    });

    Ok(fused
        .into_iter()
        .take(limit)
        .map(|(row_index, _, score)| {
            let row = &rows[row_index];
            SearchHit {
                id: row.id.clone(),
                text: row.text.clone(),
                score,
                source_kind: row.source_kind.clone(),
                source_id: row.source_id.clone(),
                created_at: row.created_at.clone(),
            }
        })
        .collect())
}

/// Hybrid top-k search for a precomputed query embedding.
///
/// Repository reads remain async and workspace-scoped. Each bounded page is
/// moved to `spawn_blocking` for BLOB decode, and the two-stage cosine+BM25
/// scoring ([`score_cached`]) also runs there, keeping CPU-heavy work off
/// Tokio executor threads. The raw query text is threaded through for the
/// keyword stage.
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
        let query_text = req.query.clone();
        let hits =
            tokio::task::spawn_blocking(move || score_cached(&query, &query_text, &cached.rows, limit))
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
        emit_changed(state, &req.workspace_id);
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
    if deleted > 0 {
        emit_changed(state, &req.workspace_id);
    }
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

/// Extract normalized `[[token]]` wiki-link tokens from one chunk's text:
/// trimmed, lowercased, de-duplicated within the chunk. A hand-rolled bracket
/// scan rather than the `regex` crate — no new dependencies (ADR 0007).
fn wiki_tokens(text: &str) -> HashSet<String> {
    let mut tokens = HashSet::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let token = after[..end].trim().to_lowercase();
        if !token.is_empty() {
            tokens.insert(token);
        }
        rest = &after[end + 2..];
    }
    tokens
}

/// `memory.graph` — nodes and derived edges for the knowledge-graph view
/// (ADR 0007). Edges are computed fresh on every call, never persisted:
///
/// - `wiki`: chunks sharing at least one identical `[[token]]`.
/// - `related`: cosine similarity between stored embeddings (already
///   L2-normalized at write time, so dot product is cosine similarity),
///   per-node top-[`RELATED_TOP_K`] at or above [`RELATED_SIMILARITY_THRESHOLD`],
///   skipping any pair already linked by `wiki`, deduped symmetrically.
///
/// An empty or missing index returns `{ nodes: [], edges: [] }`, never an
/// error.
pub async fn graph(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: WorkspaceReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;

    let Some(index) = repo::memory::get_index(&state.db, &req.workspace_id).await? else {
        return Ok(json!({ "nodes": [], "edges": [] }));
    };
    let index_dimension = usize::try_from(index.dimension).map_err(|_| {
        AppError::Invalid(format!(
            "memory index dimension {} is invalid",
            index.dimension
        ))
    })?;

    let total = repo::memory::count(&state.db, &req.workspace_id).await?;
    let rows = repo::memory::list_for_graph(&state.db, &req.workspace_id).await?;
    if total > rows.len() as i64 {
        eprintln!(
            "[memory] graph truncated workspace {} to {} of {total} chunks (cap {})",
            req.workspace_id,
            rows.len(),
            repo::memory::GRAPH_NODE_CAP
        );
    }

    let mut vectors = Vec::with_capacity(rows.len());
    let mut nodes = Vec::with_capacity(rows.len());
    let mut tokens_by_chunk = Vec::with_capacity(rows.len());
    for row in &rows {
        vectors.push(vec_codec::decode(&row.embedding, index_dimension)?);
        tokens_by_chunk.push(wiki_tokens(&row.text));
        nodes.push(json!({
            "id": row.id,
            "text": row.text,
            "sourceKind": row.source_kind,
            "sourceId": row.source_id,
            "createdAt": row.created_at,
            "updatedAt": row.updated_at,
        }));
    }

    // `wiki` edges: inverted token index, then every pair sharing a token.
    let mut token_index: HashMap<&str, Vec<usize>> = HashMap::new();
    for (chunk_index, tokens) in tokens_by_chunk.iter().enumerate() {
        for token in tokens {
            token_index.entry(token.as_str()).or_default().push(chunk_index);
        }
    }
    let mut wiki_pairs: HashSet<(usize, usize)> = HashSet::new();
    for indices in token_index.values() {
        for left in 0..indices.len() {
            for right in (left + 1)..indices.len() {
                wiki_pairs.insert((indices[left].min(indices[right]), indices[left].max(indices[right])));
            }
        }
    }

    let mut edges: Vec<Value> = wiki_pairs
        .iter()
        .map(|&(a, b)| json!({ "a": rows[a].id, "b": rows[b].id, "rel": "wiki" }))
        .collect();

    // `related` edges: per-node top-k above threshold, excluding `wiki` pairs,
    // then deduped symmetrically (keeping the higher score if both directions
    // independently selected the pair).
    let mut related_pairs: HashMap<(usize, usize), f32> = HashMap::new();
    for i in 0..rows.len() {
        let mut scored: Vec<(usize, f32)> = (0..rows.len())
            .filter(|&j| j != i && !wiki_pairs.contains(&(i.min(j), i.max(j))))
            .map(|j| {
                let score = vectors[i]
                    .iter()
                    .zip(vectors[j].iter())
                    .map(|(left, right)| left * right)
                    .sum::<f32>();
                (j, score)
            })
            .filter(|&(_, score)| score >= RELATED_SIMILARITY_THRESHOLD)
            .collect();
        scored.sort_unstable_by(|left, right| right.1.total_cmp(&left.1));
        scored.truncate(RELATED_TOP_K);
        for (j, score) in scored {
            let key = (i.min(j), i.max(j));
            related_pairs
                .entry(key)
                .and_modify(|existing| *existing = existing.max(score))
                .or_insert(score);
        }
    }
    edges.extend(
        related_pairs
            .into_iter()
            .map(|((a, b), score)| json!({ "a": rows[a].id, "b": rows[b].id, "rel": "related", "score": score })),
    );

    Ok(json!({ "nodes": nodes, "edges": edges }))
}

// ── memory review queue (plan memory-distill-queue) ──────────────────────────
//
// The distiller mines transcripts into candidate memories; a candidate becomes
// a `memory_chunk` only when a reviewer other than the proposer approves it.
// This keeps unproven auto-writes out of the semantic-search commons. Embedding
// is paid at approve time only — rejected junk never costs an embed.

/// Confirm `agent_id` is a `workspace_agent` of this workspace. Mirrors the
/// `agent` arm of [`validate_source`]: the proposer and reviewer must both be
/// real agents in the workspace, so a garbage id can never author a chunk or
/// stand in as the not-self reviewer that the gate depends on.
async fn require_workspace_agent(
    state: &AppState,
    workspace_id: &str,
    agent_id: &str,
    role: &str,
) -> Result<(), AppError> {
    let agent = repo::workspace_agent::get(&state.db, agent_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("{role} agent id={agent_id} not found")))?;
    if agent.workspace_id != workspace_id {
        return Err(AppError::Invalid(format!(
            "{role} agent does not belong to this workspace"
        )));
    }
    Ok(())
}

/// `memory.propose` — enqueue a distilled candidate for review.
///
/// No embedding is computed here (decision 1: rejected junk must not cost an
/// embed). The candidate is deduped against BOTH the review queue and the live
/// store: an already-remembered fact, or one already pending/rejected, returns
/// `{ "deduped": true }` and creates nothing.
pub async fn propose(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ProposeReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    if req.text.trim().is_empty() {
        return Err(AppError::Invalid(
            "memory proposal text must not be empty".into(),
        ));
    }
    require_workspace(state, &req.workspace_id).await?;
    require_workspace_agent(state, &req.workspace_id, &req.proposer_id, "proposer").await?;

    let hash = content_hash(&req.text);
    if repo::memory_proposal::chunk_hash_exists(&state.db, &req.workspace_id, &hash).await? {
        return Ok(json!({ "deduped": true }));
    }
    let result = repo::memory_proposal::create(
        &state.db,
        repo::memory_proposal::CreateProposalInput {
            workspace_id: &req.workspace_id,
            proposer_id: &req.proposer_id,
            text: &req.text,
            source_note: req.source_note.as_deref(),
            content_hash: &hash,
        },
    )
    .await?;
    match result.row {
        Some(row) => Ok(json!({ "id": row.id, "deduped": false })),
        None => Ok(json!({ "deduped": true })),
    }
}

/// `memory.queue` — list proposals in one state (default `pending`), newest first.
pub async fn queue(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: QueueReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    let filter = req.state.as_deref().unwrap_or("pending");
    if !matches!(filter, "pending" | "approved" | "rejected") {
        return Err(AppError::Invalid(format!(
            "unknown proposal state: {filter}; expected pending, approved, or rejected"
        )));
    }
    let proposals =
        repo::memory_proposal::list_by_state(&state.db, &req.workspace_id, filter).await?;
    Ok(json!({ "proposals": proposals }))
}

/// `memory.approve` core with an injected runtime embedder.
///
/// A pending proposal, approved by an agent OTHER than its proposer, is
/// embedded and upserted into `memory_chunk` with `source_kind='distilled'`
/// and `source_id` = the proposer (so distilled chunks stay greppable and
/// bulk-purgeable). The proposal is stamped `approved` with the new chunk id.
pub async fn approve_with_embedder(
    state: &AppState,
    payload: Value,
    embedder: Arc<dyn Embedder>,
    cache: Arc<MemorySearchCache>,
) -> Result<Value, AppError> {
    let req: ReviewReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    require_workspace_agent(state, &req.workspace_id, &req.reviewer_id, "reviewer").await?;

    let proposal = repo::memory_proposal::get(&state.db, &req.workspace_id, &req.proposal_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("proposal id={} not found", req.proposal_id)))?;
    if proposal.state != "pending" {
        return Err(AppError::Invalid(format!(
            "proposal id={} is {}, not pending",
            proposal.id, proposal.state
        )));
    }
    if proposal.proposer_id == req.reviewer_id {
        return Err(AppError::Invalid(
            "a proposer cannot approve their own proposal".into(),
        ));
    }
    if let Some(index) = repo::memory::get_index(&state.db, &req.workspace_id).await? {
        validate_embedder_identity(embedder.model_id(), embedder.dimension(), &index)?;
    }

    // Embed FIRST (pure, no DB side effects), then commit the chunk write and
    // the pending -> approved stamp in ONE transaction. A reject that wins the
    // race during the embed window makes set_reviewed match 0 rows; the chunk
    // upsert rolls back with it, so no orphan `distilled` chunk survives (F1).
    let (model_id, embedding) = embed_one(Arc::clone(&embedder), proposal.text.clone()).await?;

    let mut tx = state.db.begin().await.map_err(AppError::from)?;
    let upsert = repo::memory::upsert_chunk_on(
        &mut tx,
        UpsertChunkInput {
            workspace_id: &req.workspace_id,
            model_id,
            source_kind: "distilled",
            source_id: Some(&proposal.proposer_id),
            text: &proposal.text,
            embedding: &embedding,
            content_hash: &proposal.content_hash,
        },
    )
    .await?;
    let reviewed = repo::memory_proposal::set_reviewed(
        &mut *tx,
        &req.workspace_id,
        &proposal.id,
        "approved",
        &req.reviewer_id,
        req.reason.as_deref(),
        Some(&upsert.row.id),
    )
    .await?;
    let Some(reviewed) = reviewed else {
        tx.rollback().await.map_err(AppError::from)?;
        return Err(AppError::Invalid(format!(
            "proposal id={} is no longer pending",
            proposal.id
        )));
    };
    tx.commit().await.map_err(AppError::from)?;

    // Only after a successful commit: never invalidate the cache or emit for a
    // write that rolled back.
    cache.invalidate(&req.workspace_id);
    if !upsert.deduped {
        emit_changed(state, &req.workspace_id);
    }
    Ok(json!({
        "id": reviewed.id,
        "chunkId": upsert.row.id,
        "deduped": upsert.deduped,
    }))
}

/// `memory.approve` production wrapper.
pub async fn approve(state: &AppState, payload: Value) -> Result<Value, AppError> {
    approve_with_embedder(
        state,
        payload,
        Arc::clone(&state.memory_embedder),
        Arc::clone(&state.memory_search_cache),
    )
    .await
}

/// `memory.reject` — mark a pending proposal `rejected` with a reason.
///
/// Rejected rows are kept: their `content_hash` blocks the same fact from being
/// re-proposed on the distiller's next run. Self-rejection is allowed (a
/// proposer may retract); only self-APPROVAL is barred.
pub async fn reject(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ReviewReq =
        serde_json::from_value(payload).map_err(|error| AppError::Invalid(error.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    require_workspace_agent(state, &req.workspace_id, &req.reviewer_id, "reviewer").await?;

    let proposal = repo::memory_proposal::get(&state.db, &req.workspace_id, &req.proposal_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("proposal id={} not found", req.proposal_id)))?;
    if proposal.state != "pending" {
        return Err(AppError::Invalid(format!(
            "proposal id={} is {}, not pending",
            proposal.id, proposal.state
        )));
    }
    let reviewed = repo::memory_proposal::set_reviewed(
        &state.db,
        &req.workspace_id,
        &proposal.id,
        "rejected",
        &req.reviewer_id,
        req.reason.as_deref(),
        None,
    )
    .await?
    .ok_or_else(|| {
        AppError::Invalid(format!(
            "proposal id={} is no longer pending",
            proposal.id
        ))
    })?;
    Ok(json!({ "id": reviewed.id, "state": reviewed.state }))
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
    async fn remember_emits_memory_changed_on_a_real_insert_not_on_a_dedup() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws = fixture_workspace(&state, "emit-remember").await;

        remember_text(&state, Arc::clone(&cache), &ws, "first")
            .await
            .expect("remember");
        let after_insert = emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();
        assert_eq!(after_insert, 1, "a real insert must emit memory:changed once");

        // Re-remembering the SAME text dedupes (no row written) — must NOT
        // emit again (decision 2).
        remember_text(&state, Arc::clone(&cache), &ws, "first")
            .await
            .expect("deduped remember");
        let after_dedup = emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();
        assert_eq!(after_dedup, 1, "a deduped remember must not emit again");
    }

    #[tokio::test]
    async fn delete_emits_only_when_a_row_was_actually_deleted() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws = fixture_workspace(&state, "emit-delete").await;
        let remembered = remember_text(&state, Arc::clone(&cache), &ws, "one")
            .await
            .expect("remember");
        let count = || emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();
        let after_remember = count(); // the remember above already emitted once

        delete_with_cache(
            &state,
            json!({ "workspaceId": ws, "id": remembered["id"] }),
            Arc::clone(&cache),
        )
        .await
        .expect("delete");
        assert_eq!(
            count(),
            after_remember + 1,
            "an actual delete must emit memory:changed"
        );

        // Deleting the SAME (now-gone) id again finds nothing — must NOT
        // emit again.
        delete_with_cache(
            &state,
            json!({ "workspaceId": ws, "id": remembered["id"] }),
            Arc::clone(&cache),
        )
        .await
        .expect("delete of an already-deleted id");
        assert_eq!(
            count(),
            after_remember + 1,
            "deleting nothing must not emit again"
        );
    }

    #[tokio::test]
    async fn clear_emits_only_when_rows_were_actually_cleared() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws = fixture_workspace(&state, "emit-clear").await;

        // Clearing an already-empty workspace must not emit.
        clear_with_cache(&state, json!({ "workspaceId": ws }), Arc::clone(&cache))
            .await
            .expect("clear empty");
        let after_empty = emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();
        assert_eq!(after_empty, 0, "clearing nothing must not emit");

        remember_text(&state, Arc::clone(&cache), &ws, "one")
            .await
            .expect("remember");
        clear_with_cache(&state, json!({ "workspaceId": ws }), Arc::clone(&cache))
            .await
            .expect("clear populated");
        // remember (1) + clear (1) = 2 emits total for this workspace.
        let after_populated = emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();
        assert_eq!(after_populated, 2, "clearing an actual row must emit");
    }

    #[tokio::test]
    async fn approve_emits_on_a_real_chunk_write_propose_and_reject_never_emit() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws = fixture_workspace(&state, "emit-approve").await;
        let proposer = fixture_agent(&state, &ws, "Proposer").await;
        let reviewer = fixture_agent(&state, &ws, "Reviewer").await;

        let proposed = propose(
            &state,
            json!({
                "workspaceId": ws,
                "proposerId": proposer,
                "text": "a distilled fact",
            }),
        )
        .await
        .expect("propose");
        let emits_after_propose = emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();
        assert_eq!(emits_after_propose, 0, "propose never touches memory_chunk — must not emit");

        approve_with_embedder(
            &state,
            json!({ "workspaceId": ws, "reviewerId": reviewer, "proposalId": proposed["id"] }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect("approve");
        let emits_after_approve = emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();
        assert_eq!(emits_after_approve, 1, "approve writes a chunk — must emit");

        // A second proposal that gets REJECTED must never emit either.
        let proposed2 = propose(
            &state,
            json!({
                "workspaceId": ws,
                "proposerId": proposer,
                "text": "a different distilled fact",
            }),
        )
        .await
        .expect("propose 2");
        reject(
            &state,
            json!({ "workspaceId": ws, "reviewerId": reviewer, "proposalId": proposed2["id"] }),
        )
        .await
        .expect("reject");
        let emits_after_reject = emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();
        assert_eq!(emits_after_reject, 1, "reject never touches memory_chunk — must not emit");
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

    // ── memory review queue (plan memory-distill-queue) ──────────────────

    async fn fixture_agent(state: &AppState, workspace_id: &str, name: &str) -> String {
        let definition = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: name.into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create definition");
        workspace_agent::instantiate(&state.db, workspace_id, &definition.id)
            .await
            .expect("instantiate agent")
            .id
    }

    #[tokio::test]
    async fn propose_then_approve_stores_distilled_chunk_and_is_searchable() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws = fixture_workspace(&state, "distill-happy").await;
        let proposer = fixture_agent(&state, &ws, "Proposer").await;
        let reviewer = fixture_agent(&state, &ws, "Reviewer").await;

        let proposed = propose(
            &state,
            json!({
                "workspaceId": ws,
                "proposerId": proposer,
                "text": "failed approach: rusqlite was rejected for sqlx",
                "sourceNote": "transcript.jsonl 2026-07-05",
            }),
        )
        .await
        .expect("propose");
        assert_eq!(proposed["deduped"], false);
        let proposal_id = proposed["id"].as_str().unwrap().to_owned();

        let approved = approve_with_embedder(
            &state,
            json!({ "workspaceId": ws, "reviewerId": reviewer, "proposalId": proposal_id }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect("approve");
        assert_eq!(approved["deduped"], false);
        let chunk_id = approved["chunkId"].as_str().unwrap().to_owned();

        // Pending queue is now empty; the proposal moved to approved carrying
        // the chunk id and the reviewer.
        let pending = queue(&state, json!({ "workspaceId": ws }))
            .await
            .expect("queue pending");
        assert!(pending["proposals"].as_array().unwrap().is_empty());
        let approved_list = queue(&state, json!({ "workspaceId": ws, "state": "approved" }))
            .await
            .expect("queue approved");
        let rows = approved_list["proposals"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["chunkId"], chunk_id);
        assert_eq!(rows[0]["state"], "approved");
        assert_eq!(rows[0]["reviewerId"], reviewer);

        // The chunk is searchable and stored as a `distilled` chunk sourced to
        // the proposer (greppable + bulk-purgeable, decision 4).
        let result = search_with_embedder(
            &state,
            json!({ "workspaceId": ws, "query": "rusqlite sqlx" }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect("search");
        let hits = result["hits"].as_array().unwrap();
        assert!(hits.iter().any(|hit| {
            hit["id"] == json!(chunk_id)
                && hit["sourceKind"] == "distilled"
                && hit["sourceId"] == json!(proposer)
        }));
    }

    #[tokio::test]
    async fn self_approve_is_rejected_before_any_chunk_is_written() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws = fixture_workspace(&state, "distill-self").await;
        let proposer = fixture_agent(&state, &ws, "Solo").await;
        let proposed = propose(
            &state,
            json!({ "workspaceId": ws, "proposerId": proposer, "text": "self review attempt" }),
        )
        .await
        .expect("propose");
        let proposal_id = proposed["id"].as_str().unwrap().to_owned();

        let error = approve_with_embedder(
            &state,
            json!({ "workspaceId": ws, "reviewerId": proposer, "proposalId": proposal_id }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect_err("self-approve must fail");
        assert!(
            matches!(error, AppError::Invalid(message) if message.contains("cannot approve their own"))
        );

        // The gate fired before embedding: no chunk exists, proposal stays pending.
        let status = status_with_embedder(
            &state,
            json!({ "workspaceId": ws }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
        )
        .await
        .expect("status");
        assert_eq!(status["chunks"], 0);
        let pending = queue(&state, json!({ "workspaceId": ws }))
            .await
            .expect("queue");
        assert_eq!(pending["proposals"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reject_stamps_reason_and_a_second_review_errors() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state, "distill-reject").await;
        let proposer = fixture_agent(&state, &ws, "P").await;
        let reviewer = fixture_agent(&state, &ws, "R").await;
        let proposed = propose(
            &state,
            json!({ "workspaceId": ws, "proposerId": proposer, "text": "candidate to reject" }),
        )
        .await
        .expect("propose");
        let proposal_id = proposed["id"].as_str().unwrap().to_owned();

        let rejected = reject(
            &state,
            json!({
                "workspaceId": ws,
                "reviewerId": reviewer,
                "proposalId": proposal_id,
                "reason": "already in the docs",
            }),
        )
        .await
        .expect("reject");
        assert_eq!(rejected["state"], "rejected");

        let rejected_list = queue(&state, json!({ "workspaceId": ws, "state": "rejected" }))
            .await
            .expect("queue rejected");
        let rows = rejected_list["proposals"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["reviewReason"], "already in the docs");

        // A rejected proposal is no longer pending: re-review errors on both verbs.
        let reject_again = reject(
            &state,
            json!({ "workspaceId": ws, "reviewerId": reviewer, "proposalId": proposal_id }),
        )
        .await
        .expect_err("re-reject must error");
        assert!(matches!(reject_again, AppError::Invalid(message) if message.contains("not pending")));
        let approve_rejected = approve_with_embedder(
            &state,
            json!({ "workspaceId": ws, "reviewerId": reviewer, "proposalId": proposal_id }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::new(MemorySearchCache::new()),
        )
        .await
        .expect_err("approve of rejected must error");
        assert!(matches!(approve_rejected, AppError::Invalid(message) if message.contains("not pending")));
    }

    /// Embedder that lands a reject on the target proposal DURING `embed`, so
    /// the proposal is pending at the approve precondition check but no longer
    /// pending when the approve transaction runs `set_reviewed` — a
    /// deterministic simulation of the F1 approve/reject race.
    struct RejectDuringEmbed {
        inner: FakeEmbedder,
        pool: SqlitePool,
        workspace_id: String,
        proposal_id: String,
    }

    impl Embedder for RejectDuringEmbed {
        fn model_id(&self) -> &'static str {
            self.inner.model_id()
        }

        fn dimension(&self) -> usize {
            self.inner.dimension()
        }

        fn is_ready(&self) -> bool {
            true
        }

        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            // Runs on a spawn_blocking thread (see embed_one), so blocking on
            // the runtime handle here is safe and does not stall the executor.
            let pool = self.pool.clone();
            let ws = self.workspace_id.clone();
            let id = self.proposal_id.clone();
            tokio::runtime::Handle::current().block_on(async move {
                repo::memory_proposal::set_reviewed(
                    &pool,
                    &ws,
                    &id,
                    "rejected",
                    "racing-reviewer",
                    None,
                    None,
                )
                .await
                .expect("racing reject lands");
            });
            self.inner.embed(texts)
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn approve_rolls_back_and_does_not_emit_when_a_reject_wins_the_race() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws = fixture_workspace(&state, "distill-race").await;
        let proposer = fixture_agent(&state, &ws, "P").await;
        let reviewer = fixture_agent(&state, &ws, "R").await;
        let proposed = propose(
            &state,
            json!({ "workspaceId": ws, "proposerId": proposer, "text": "raced distilled fact" }),
        )
        .await
        .expect("propose");
        let proposal_id = proposed["id"].as_str().unwrap().to_owned();

        let baseline = emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();

        let embedder = Arc::new(RejectDuringEmbed {
            inner: FakeEmbedder::new(DIMENSION),
            pool: state.db.clone(),
            workspace_id: ws.clone(),
            proposal_id: proposal_id.clone(),
        });
        let error = approve_with_embedder(
            &state,
            json!({ "workspaceId": ws, "reviewerId": reviewer, "proposalId": proposal_id }),
            embedder,
            Arc::clone(&cache),
        )
        .await
        .expect_err("a racing reject must make approve fail");
        assert!(
            matches!(error, AppError::Invalid(message) if message.contains("no longer pending"))
        );

        // The chunk upsert rolled back with the failed stamp: no chunk exists.
        let status = status_with_embedder(
            &state,
            json!({ "workspaceId": ws }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
        )
        .await
        .expect("status");
        assert_eq!(status["chunks"], 0, "a rolled-back approve leaves no chunk (F1)");

        // The racer's reject stands; nothing landed in `approved`.
        let rejected = queue(&state, json!({ "workspaceId": ws, "state": "rejected" }))
            .await
            .expect("queue rejected");
        assert_eq!(rejected["proposals"].as_array().unwrap().len(), 1);
        let approved = queue(&state, json!({ "workspaceId": ws, "state": "approved" }))
            .await
            .expect("queue approved");
        assert!(approved["proposals"].as_array().unwrap().is_empty());

        // No memory:changed emitted for the write that rolled back.
        let after = emit_probe().lock().unwrap().iter().filter(|w| *w == &ws).count();
        assert_eq!(
            after, baseline,
            "a rolled-back approve must not emit memory:changed"
        );
    }

    #[tokio::test]
    async fn propose_dedups_against_queue_and_live_store() {
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let ws = fixture_workspace(&state, "distill-dedup").await;
        let proposer = fixture_agent(&state, &ws, "P").await;

        // vs an existing proposal: identical text creates nothing the second time.
        let first = propose(
            &state,
            json!({ "workspaceId": ws, "proposerId": proposer, "text": "duplicate fact" }),
        )
        .await
        .expect("first propose");
        assert_eq!(first["deduped"], false);
        let dup = propose(
            &state,
            json!({ "workspaceId": ws, "proposerId": proposer, "text": "duplicate fact" }),
        )
        .await
        .expect("dup propose");
        assert_eq!(dup["deduped"], true);
        assert!(dup.get("id").is_none());
        assert_eq!(
            queue(&state, json!({ "workspaceId": ws })).await.unwrap()["proposals"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        // vs the live store: a remembered fact blocks proposing the same text.
        remember_text(&state, Arc::clone(&cache), &ws, "already remembered fact")
            .await
            .expect("remember");
        let vs_chunk = propose(
            &state,
            json!({ "workspaceId": ws, "proposerId": proposer, "text": "already remembered fact" }),
        )
        .await
        .expect("propose vs live chunk");
        assert_eq!(vs_chunk["deduped"], true);
        assert!(vs_chunk.get("id").is_none());
        assert_eq!(
            queue(&state, json!({ "workspaceId": ws })).await.unwrap()["proposals"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn router_dispatches_review_queue_commands() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state, "distill-router").await;
        let proposer = fixture_agent(&state, &ws, "P").await;
        let reviewer = fixture_agent(&state, &ws, "R").await;

        let proposed = router::dispatch(
            &state,
            "memory.propose",
            json!({ "workspaceId": ws, "proposerId": proposer, "text": "router distill" }),
        )
        .await
        .expect("propose route");
        let proposal_id = proposed["id"].as_str().unwrap().to_owned();

        let queued = router::dispatch(&state, "memory.queue", json!({ "workspaceId": ws }))
            .await
            .expect("queue route");
        assert_eq!(queued["proposals"].as_array().unwrap().len(), 1);

        let approved = router::dispatch(
            &state,
            "memory.approve",
            json!({ "workspaceId": ws, "reviewerId": reviewer, "proposalId": proposal_id }),
        )
        .await
        .expect("approve route");
        assert!(approved["chunkId"].is_string());

        let second = router::dispatch(
            &state,
            "memory.propose",
            json!({ "workspaceId": ws, "proposerId": proposer, "text": "router distill two" }),
        )
        .await
        .expect("second propose route");
        let rejected = router::dispatch(
            &state,
            "memory.reject",
            json!({ "workspaceId": ws, "reviewerId": reviewer, "proposalId": second["id"] }),
        )
        .await
        .expect("reject route");
        assert_eq!(rejected["state"], "rejected");
    }

    // ── memory.graph (ADR 0007) ──────────────────────────────────────────

    fn basis_vector(index: usize) -> Vec<f32> {
        let mut vector = vec![0.0f32; DIMENSION];
        vector[index] = 1.0;
        vector
    }

    async fn insert_chunk(
        state: &AppState,
        workspace_id: &str,
        text: &str,
        embedding: &[f32],
        content_hash: &str,
    ) -> memory::UpsertChunkResult {
        memory::upsert_chunk(
            &state.db,
            UpsertChunkInput {
                workspace_id,
                model_id: MODEL,
                source_kind: "manual",
                source_id: None,
                text,
                embedding,
                content_hash,
            },
        )
        .await
        .expect("upsert chunk fixture")
    }

    #[tokio::test]
    async fn graph_empty_index_returns_empty_arrays_not_error() {
        let state = AppState::for_tests().await;
        let workspace_id = fixture_workspace(&state, "graph-empty").await;

        let result =
            router::dispatch(&state, "memory.graph", json!({ "workspaceId": workspace_id }))
                .await
                .expect("graph on empty index");
        assert_eq!(result, json!({ "nodes": [], "edges": [] }));
    }

    #[tokio::test]
    async fn graph_derives_wiki_edges_case_insensitively_and_ignores_untokened_chunks() {
        let state = AppState::for_tests().await;
        let workspace_id = fixture_workspace(&state, "graph-wiki").await;

        // Orthogonal embeddings hold `related` cosine similarity at exactly 0
        // for every pair, isolating this test to `wiki`-edge derivation.
        let a = insert_chunk(&state, &workspace_id, "loves [[Alpha]] concept", &basis_vector(0), "wiki-a").await;
        let b = insert_chunk(&state, &workspace_id, "shares [[alpha]] too", &basis_vector(1), "wiki-b").await;
        insert_chunk(&state, &workspace_id, "no tokens here", &basis_vector(2), "wiki-c").await;

        let result =
            router::dispatch(&state, "memory.graph", json!({ "workspaceId": workspace_id }))
                .await
                .expect("graph");
        assert_eq!(result["nodes"].as_array().unwrap().len(), 3);

        let edges = result["edges"].as_array().unwrap();
        assert_eq!(
            edges.len(),
            1,
            "only a/b share a token; orthogonal embeddings score 0 for `related`: {edges:?}"
        );
        assert_eq!(edges[0]["rel"], "wiki");
        assert!(edges[0].get("score").is_none(), "wiki edges carry no score");
        let endpoints: HashSet<&str> = [
            edges[0]["a"].as_str().unwrap(),
            edges[0]["b"].as_str().unwrap(),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            endpoints,
            HashSet::from([a.row.id.as_str(), b.row.id.as_str()])
        );
    }

    #[tokio::test]
    async fn graph_related_edges_skip_wiki_pairs_and_score_cosine_similarity() {
        let state = AppState::for_tests().await;
        let workspace_id = fixture_workspace(&state, "graph-related").await;

        // p/q are wiki-linked AND identical in direction (cosine 1.0) — the
        // `related` edge between them must be suppressed. r shares no token
        // but is also identical in direction, so it gets `related` edges to
        // both p and q. s is orthogonal to everything and stays isolated.
        let p = insert_chunk(&state, &workspace_id, "[[shared]] one", &basis_vector(0), "rel-p").await;
        let q = insert_chunk(&state, &workspace_id, "[[shared]] two", &basis_vector(0), "rel-q").await;
        let r = insert_chunk(&state, &workspace_id, "no tokens, same direction", &basis_vector(0), "rel-r").await;
        insert_chunk(&state, &workspace_id, "no tokens, orthogonal", &basis_vector(1), "rel-s").await;

        let result =
            router::dispatch(&state, "memory.graph", json!({ "workspaceId": workspace_id }))
                .await
                .expect("graph");
        let edges = result["edges"].as_array().unwrap().clone();
        assert_eq!(edges.len(), 3, "{edges:?}");

        let wiki: Vec<&Value> = edges.iter().filter(|edge| edge["rel"] == "wiki").collect();
        assert_eq!(wiki.len(), 1);
        let wiki_endpoints: HashSet<&str> = [
            wiki[0]["a"].as_str().unwrap(),
            wiki[0]["b"].as_str().unwrap(),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            wiki_endpoints,
            HashSet::from([p.row.id.as_str(), q.row.id.as_str()])
        );

        let related: Vec<&Value> = edges.iter().filter(|edge| edge["rel"] == "related").collect();
        assert_eq!(related.len(), 2, "{related:?}");
        for edge in &related {
            let score = edge["score"].as_f64().expect("related edge carries a score");
            assert!(
                (score - 1.0).abs() < 1e-4,
                "identical unit vectors must score ~1.0 cosine, got {score}"
            );
            let endpoints: HashSet<&str> =
                [edge["a"].as_str().unwrap(), edge["b"].as_str().unwrap()]
                    .into_iter()
                    .collect();
            assert!(
                endpoints.contains(r.row.id.as_str()),
                "both related edges must touch r: {endpoints:?}"
            );
            assert!(
                !(endpoints.contains(p.row.id.as_str()) && endpoints.contains(q.row.id.as_str())),
                "p-q must stay wiki-only, never duplicated as related"
            );
        }
    }

    #[tokio::test]
    async fn graph_related_edges_cap_at_top_k_per_node() {
        let state = AppState::for_tests().await;
        let workspace_id = fixture_workspace(&state, "graph-topk").await;

        // All vectors live in the (dim0, dim1) unit circle, so cosine
        // similarity between any two is exactly cos(angle difference).
        // Clusters (b1,b2)/(c1,c2)/(d1,d2)/(e1,e2) are each mutually closer
        // than any of them are to `a`, so none of their own top-K picks
        // reach back to `a` — any edge touching `a` can only come from `a`'s
        // own top-RELATED_TOP_K selection among all candidates above
        // threshold.
        let angled = |degrees: f64| -> Vec<f32> {
            let radians = degrees.to_radians();
            let mut vector = vec![0.0f32; DIMENSION];
            vector[0] = radians.cos() as f32;
            vector[1] = radians.sin() as f32;
            vector
        };

        let a = insert_chunk(&state, &workspace_id, "a", &angled(0.0), "topk-a").await;
        let b1 = insert_chunk(&state, &workspace_id, "b1", &angled(6.0), "topk-b1").await;
        let b2 = insert_chunk(&state, &workspace_id, "b2", &angled(7.0), "topk-b2").await;
        let c1 = insert_chunk(&state, &workspace_id, "c1", &angled(20.0), "topk-c1").await;
        insert_chunk(&state, &workspace_id, "c2", &angled(21.0), "topk-c2").await;
        let d1 = insert_chunk(&state, &workspace_id, "d1", &angled(34.0), "topk-d1").await;
        let d2 = insert_chunk(&state, &workspace_id, "d2", &angled(35.0), "topk-d2").await;
        let e1 = insert_chunk(&state, &workspace_id, "e1", &angled(48.0), "topk-e1").await;
        let e2 = insert_chunk(&state, &workspace_id, "e2", &angled(49.0), "topk-e2").await;

        let result =
            router::dispatch(&state, "memory.graph", json!({ "workspaceId": workspace_id }))
                .await
                .expect("graph");
        let edges = result["edges"].as_array().unwrap();

        let a_id = json!(a.row.id);
        let a_edges: Vec<&Value> = edges
            .iter()
            .filter(|edge| edge["a"] == a_id || edge["b"] == a_id)
            .collect();
        assert_eq!(
            a_edges.len(),
            RELATED_TOP_K,
            "node a must cap at its own top-{RELATED_TOP_K}, got {a_edges:?}"
        );

        let a_neighbors: HashSet<&str> = a_edges
            .iter()
            .flat_map(|edge| [edge["a"].as_str().unwrap(), edge["b"].as_str().unwrap()])
            .filter(|id| *id != a.row.id)
            .collect();
        assert_eq!(
            a_neighbors,
            HashSet::from([b1.row.id.as_str(), b2.row.id.as_str(), c1.row.id.as_str()]),
            "a's top-3 by cosine must be its 3 closest angles (b1, b2, c1)"
        );
        for excluded in [&d1.row.id, &d2.row.id, &e1.row.id, &e2.row.id] {
            assert!(
                !a_neighbors.contains(excluded.as_str()),
                "farther clusters must not reach back to a via their own top-k: {excluded}"
            );
        }
    }

    #[test]
    fn cosine_candidates_bounded_heap_matches_full_sort_reference() {
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

        let actual: Vec<(String, f32)> = cosine_candidates(&query, &rows, 7)
            .expect("cosine candidates")
            .into_iter()
            .map(|hit| (hit.id, hit.score))
            .collect();
        assert_eq!(actual, expected[..7]);
    }

    fn cached_row(id: &str, text: &str, vector: Vec<f32>) -> CachedMemory {
        CachedMemory {
            id: id.into(),
            text: text.into(),
            vector,
            source_kind: "manual".into(),
            source_id: None,
            created_at: "2026-07-04T00:00:00Z".into(),
        }
    }

    #[test]
    fn hybrid_lifts_token_match_above_higher_cosine() {
        // `higher` wins on pure cosine (dot 1.0 vs 0.6) but shares no query
        // token; `lower` shares the rare token, so the BM25 stage must flip
        // them: fused(lower) = 0.6·0.6 + 0.4·1.0 = 0.76 > fused(higher) = 0.6.
        let query = vec![1.0f32, 0.0];
        let rows = vec![
            cached_row("higher", "beta zebra", vec![1.0, 0.0]),
            cached_row("lower", "alpha unicorn", vec![0.6, 0.8]),
        ];
        let hits = score_cached(&query, "unicorn", &rows, 2).expect("hybrid score");
        assert_eq!(hits[0].id, "lower", "token match must outrank higher cosine");
        assert_eq!(hits[1].id, "higher");
        assert!(hits[0].score > hits[1].score);

        // With no shared token the keyword term is zero, so pure cosine order
        // stands (fused = 0.6·cosine, a monotonic scaling).
        let plain = score_cached(&query, "gamma", &rows, 2).expect("hybrid score");
        assert_eq!(plain[0].id, "higher");
        assert_eq!(plain[1].id, "lower");
    }

    #[test]
    fn hybrid_ranking_is_deterministic() {
        let query = vec![0.5f32, 0.5];
        let rows = vec![
            cached_row("a", "shared token alpha", vec![1.0, 0.0]),
            cached_row("b", "shared token beta", vec![0.0, 1.0]),
            cached_row("c", "unrelated gamma", vec![0.5, 0.5]),
        ];
        let first = score_cached(&query, "shared token", &rows, 3).expect("first");
        let second = score_cached(&query, "shared token", &rows, 3).expect("second");
        let ids = |hits: &[SearchHit]| hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&first), ids(&second), "same store + query ⇒ same ranking");
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

    // ── T6 validation: real model, offline behavior, corrupt/missing cache ──
    //
    // These use the REAL FastembedEmbedder (not FakeEmbedder) against a
    // dedicated on-disk cache dir and are NOT part of the normal
    // `cargo test --lib` gate. Run manually, in order — see each test's
    // `#[ignore]` message for the exact invocation.

    fn t6_validation_cache_dir() -> std::path::PathBuf {
        std::env::temp_dir().join("conclave-t6-validation-cache")
    }

    #[tokio::test]
    #[ignore = "downloads the real MiniLM model on first run (network required); run \
                BEFORE t6_search_and_status_work_offline; invoke: cargo test --lib \
                commands::memory::tests::t6_remember_downloads_model_once -- --ignored --nocapture"]
    async fn t6_remember_downloads_model_once() {
        let cache_dir = t6_validation_cache_dir();
        let _ = std::fs::remove_dir_all(&cache_dir);
        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let embedder: Arc<dyn Embedder> = Arc::new(
            crate::engine::runtime::embedder::FastembedEmbedder::new(cache_dir.clone()),
        );
        let workspace_id = fixture_workspace(&state, "t6-offline").await;

        assert!(!embedder.is_ready(), "cold cache must not report ready");

        let result = remember_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id, "text": "remember this offline" }),
            Arc::clone(&embedder),
            Arc::clone(&cache),
        )
        .await
        .expect("remember should succeed (downloads the model on first use)");
        assert!(result.get("id").is_some());
        assert!(
            embedder.is_ready(),
            "model must report ready after a successful embed"
        );
        println!("[T6] remember downloaded the model and stored one chunk");
    }

    #[tokio::test]
    #[ignore = "run AFTER t6_remember_downloads_model_once (same fixed cache dir must \
                already be warm); invoke with the WHOLE PROCESS offline to prove no \
                network dependency remains: HTTP_PROXY=http://127.0.0.1:1 \
                HTTPS_PROXY=http://127.0.0.1:1 cargo test --lib \
                commands::memory::tests::t6_search_and_status_work_offline -- --ignored --nocapture"]
    async fn t6_search_and_status_work_offline() {
        let cache_dir = t6_validation_cache_dir();
        assert!(
            cache_dir.join("models--Xenova--all-MiniLM-L6-v2").exists(),
            "run t6_remember_downloads_model_once first to warm this cache dir"
        );

        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let embedder: Arc<dyn Embedder> =
            Arc::new(crate::engine::runtime::embedder::FastembedEmbedder::new(cache_dir));
        let workspace_id = fixture_workspace(&state, "t6-offline-search").await;

        // Fresh in-memory DB (AppState::for_tests), so this must re-remember
        // in THIS process — still must be network-free, since the model
        // itself is what's cached, not any particular remembered text.
        remember_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id, "text": "offline remember" }),
            Arc::clone(&embedder),
            Arc::clone(&cache),
        )
        .await
        .expect("remember must succeed fully offline once the model is cached");

        let hits = search_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id, "query": "offline" }),
            Arc::clone(&embedder),
            Arc::clone(&cache),
        )
        .await
        .expect("search must succeed fully offline");
        assert!(!hits["hits"].as_array().expect("hits array").is_empty());

        let status = status_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id }),
            Arc::clone(&embedder),
        )
        .await
        .expect("status must succeed fully offline");
        assert_eq!(status["modelReady"], json!(true));
        println!("[T6] remember/search/status all succeeded fully offline");
    }

    #[tokio::test]
    #[ignore = "run AFTER t6_remember_downloads_model_once (needs a warm cache dir); \
                deletes then corrupts a cached model file and checks recovery \
                behavior; invoke: cargo test --lib commands::memory::tests::\
                t6_corrupt_and_missing_model_file_recover_cleanly -- --ignored --nocapture"]
    async fn t6_corrupt_and_missing_model_file_recover_cleanly() {
        let cache_dir = t6_validation_cache_dir();
        let repo_dir = cache_dir.join("models--Xenova--all-MiniLM-L6-v2");
        assert!(
            repo_dir.exists(),
            "run t6_remember_downloads_model_once first to warm this cache dir"
        );

        // ── DELETE: the whole cache is gone -> modelReady must go false,
        // and the status handler itself must not error or hang.
        let backup = cache_dir.with_extension("bak");
        let _ = std::fs::remove_dir_all(&backup);
        std::fs::rename(&cache_dir, &backup).expect("move cache dir aside");

        let state = AppState::for_tests().await;
        let cache = Arc::new(MemorySearchCache::new());
        let deleted_embedder: Arc<dyn Embedder> = Arc::new(
            crate::engine::runtime::embedder::FastembedEmbedder::new(cache_dir.clone()),
        );
        assert!(
            !deleted_embedder.is_ready(),
            "a deleted cache must report modelReady=false"
        );

        let workspace_id = fixture_workspace(&state, "t6-corrupt").await;
        let status = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            status_with_embedder(
                &state,
                json!({ "workspaceId": workspace_id }),
                Arc::clone(&deleted_embedder),
            ),
        )
        .await
        .expect("status must not hang with a missing model cache")
        .expect("status handler itself must not error even with a missing model");
        assert_eq!(status["modelReady"], json!(false));
        println!("[T6] deleted cache: modelReady=false, status did not error or hang");

        // ── restore, then CORRUPT one blob file in place (files present,
        // content garbage) ──────────────────────────────────────────────
        std::fs::rename(&backup, &cache_dir).expect("restore cache dir");
        let blobs_dir = repo_dir.join("blobs");
        let blob = std::fs::read_dir(&blobs_dir)
            .expect("read blobs dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .find(|path| path.is_file() && !path.to_string_lossy().ends_with(".lock"))
            .expect("at least one non-lock blob file in the warm cache");
        let original = std::fs::read(&blob).expect("read original blob");
        std::fs::write(&blob, b"not a valid onnx model, corrupted for T6 validation")
            .expect("corrupt blob");

        let corrupt_embedder: Arc<dyn Embedder> = Arc::new(
            crate::engine::runtime::embedder::FastembedEmbedder::new(cache_dir.clone()),
        );
        // Documented behavior (is_model_downloaded is an existence check, not
        // an integrity check): files are present, so a corrupt-but-present
        // cache still reports ready. The real failure surfaces on the next
        // embed call, checked below — NOT as modelReady=false.
        assert!(
            corrupt_embedder.is_ready(),
            "corrupt-but-present cache still reports ready (by design, see is_model_downloaded doc)"
        );

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            remember_with_embedder(
                &state,
                json!({ "workspaceId": workspace_id, "text": "should fail cleanly" }),
                Arc::clone(&corrupt_embedder),
                Arc::clone(&cache),
            ),
        )
        .await
        .expect("remember against a corrupted model must not hang");
        assert!(
            result.is_err(),
            "a corrupted model file must fail loudly, not silently succeed"
        );
        println!(
            "[T6] corrupted blob: remember failed cleanly (not hung) with {:?}",
            result.unwrap_err()
        );

        // Leave the fixture cache dir valid for a later manual rerun.
        std::fs::write(&blob, original).expect("restore original blob");
    }
}

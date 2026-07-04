//! Embedding seam for the workspace memory system (memory-v1 contract).
//!
//! This module holds the CONTRACT between the memory command handlers (T4)
//! and the production embedding backend (T3): the [`Embedder`] trait plus the
//! deterministic [`FakeEmbedder`] test double. It was landed by the lead so
//! both lanes can build against it independently; `FastembedEmbedder` (T3)
//! is added to this module by its lane, and nothing else edits it.
//!
//! # Contract (docs/2026-07-04-plan-workspace-memory-v1.md — FIXED)
//!
//! `embed` is blocking; callers wrap it in `tokio::task::spawn_blocking`
//! (global constraint 5). Returned vectors are L2-normalized, one per input
//! text, in input order, each exactly `dimension()` long — so cosine
//! similarity is a plain dot product downstream (global constraint 3).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

/// Embedding failure, kept separate from `AppError` so the trait stays free
/// of command-layer concerns; handlers map it at the boundary.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// The model is not present locally and could not be fetched.
    #[error("embedding model unavailable: {0}")]
    ModelUnavailable(String),

    /// The model ran but inference failed.
    #[error("embedding failed: {0}")]
    Failed(String),
}

/// Production/test seam for turning text into normalized vectors.
pub trait Embedder: Send + Sync {
    /// Stable identifier recorded in `memory_index.model_id`.
    fn model_id(&self) -> &'static str;

    /// Vector width; must match `memory_index.dimension`.
    fn dimension(&self) -> usize;

    /// Whether `embed` would succeed right now without network access:
    /// the model is initialized in memory, or its files are fully present
    /// in the local cache. Backs `memory.status.modelReady`.
    ///
    /// MUST be cheap and side-effect free — never triggers download or
    /// model initialization. `false` is a state ("not downloaded yet"),
    /// not an error.
    fn is_ready(&self) -> bool;

    /// Blocking; caller wraps in `spawn_blocking`. Returns one normalized
    /// f32 vector per input text, in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}

/// Deterministic, dependency-free [`Embedder`] for tests and benchmarks.
///
/// Same text → same vector (seeded from a hash of the text), already
/// L2-normalized. NOT semantically meaningful — it exists so repo/search
/// lanes can compile, test, and benchmark without the real model.
pub struct FakeEmbedder {
    dimension: usize,
}

impl FakeEmbedder {
    pub fn new(dimension: usize) -> Self {
        assert!(dimension > 0, "FakeEmbedder dimension must be non-zero");
        Self { dimension }
    }
}

impl Embedder for FakeEmbedder {
    fn model_id(&self) -> &'static str {
        "fake-embedder"
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        texts
            .iter()
            .map(|text| {
                let mut vector: Vec<f32> = (0..self.dimension)
                    .map(|i| {
                        let mut hasher = DefaultHasher::new();
                        text.hash(&mut hasher);
                        i.hash(&mut hasher);
                        // Map the hash onto [-1, 1); deterministic per (text, i).
                        (hasher.finish() as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32
                    })
                    .collect();
                let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                if norm == 0.0 {
                    return Err(EmbedError::Failed(format!(
                        "degenerate zero vector for text {text:?}"
                    )));
                }
                for value in &mut vector {
                    *value /= norm;
                }
                Ok(vector)
            })
            .collect()
    }
}

/// Output dimension of `AllMiniLML6V2Q` (plan R2).
pub const MINILM_L6_V2_Q_DIMENSION: usize = 384;

/// Model identity recorded in `memory_index.model_id` for the production model.
pub const MINILM_L6_V2_Q_MODEL_ID: &str = "all-minilm-l6-v2-q8";

/// `~/Library/Application Support/Conclave/models` (macOS) — mirrors the
/// `dirs::data_dir()`/`Conclave` convention in `engine/db.rs::db_path`,
/// scoped to a `models` subdir so it never collides with `conclave.db`.
pub fn model_cache_dir() -> PathBuf {
    dirs::data_dir()
        .expect("could not resolve user data directory")
        .join("Conclave")
        .join("models")
}

/// Best-effort check for whether the model has already been downloaded to
/// `cache_dir`, without loading it into memory. Backs
/// [`FastembedEmbedder`]'s [`Embedder::is_ready`] disk-cache branch (an
/// in-process session load is the other) — `pub` so it can also be
/// unit-tested directly without constructing a full `FastembedEmbedder`.
/// Works across process restarts, before any embed call has happened in the
/// current process.
///
/// This inspects the `hf-hub` cache layout for the pinned repo
/// (`models--Xenova--all-MiniLM-L6-v2`) rather than the model's own manifest,
/// so it stays a directory-existence check, not a hash/integrity check — a
/// corrupt-but-present cache is still reported ready and surfaces its real
/// error on the next `embed` call.
pub fn is_model_downloaded(cache_dir: &Path) -> bool {
    let repo_dir = cache_dir.join("models--Xenova--all-MiniLM-L6-v2");
    let Ok(mut entries) = std::fs::read_dir(&repo_dir) else {
        return false;
    };
    entries.next().is_some()
}

/// Production [`Embedder`] backed by `fastembed`'s quantized MiniLM model.
///
/// The ONNX session is loaded lazily on the first [`Embedder::embed`] call
/// (triggering a one-time download if the cache is cold) and cached in a
/// `Mutex` for the process lifetime — `fastembed::TextEmbedding::embed` takes
/// `&mut self`, so the mutex supplies the interior mutability the `&self`
/// trait signature requires.
pub struct FastembedEmbedder {
    cache_dir: PathBuf,
    session: OnceLock<Mutex<TextEmbedding>>,
    /// Serializes the fallible first init. `OnceLock::get_or_init` cannot be
    /// used here because loading the model can fail and `get_or_try_init` is
    /// still unstable (`once_cell_try`) — without this lock, two concurrent
    /// cold `embed` calls would both observe `session.get() == None` and
    /// both run `TextEmbedding::try_new` (double download + double model
    /// load into the same cache dir). Holding this lock across the whole
    /// check-then-init means only one caller actually initializes; the rest
    /// block, then re-check `session.get()` and find it already populated.
    init_lock: Mutex<()>,
}

impl FastembedEmbedder {
    /// Construct an embedder that caches the model under `cache_dir`. Does
    /// not touch the filesystem or network until the first [`Embedder::embed`]
    /// call.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            session: OnceLock::new(),
            init_lock: Mutex::new(()),
        }
    }

    /// Construct an embedder using the Conclave-scoped model cache
    /// directory ([`model_cache_dir`]).
    pub fn with_default_cache_dir() -> Self {
        Self::new(model_cache_dir())
    }

    fn session(&self) -> Result<&Mutex<TextEmbedding>, EmbedError> {
        if let Some(session) = self.session.get() {
            return Ok(session);
        }

        let _init_guard = self
            .init_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Re-check: another thread may have finished initializing while we
        // were waiting for `init_lock`.
        if let Some(session) = self.session.get() {
            return Ok(session);
        }

        std::fs::create_dir_all(&self.cache_dir).map_err(|error| {
            EmbedError::ModelUnavailable(format!(
                "could not create model cache dir {:?}: {error}",
                self.cache_dir
            ))
        })?;

        let options = TextInitOptions::new(EmbeddingModel::AllMiniLML6V2Q)
            .with_cache_dir(self.cache_dir.clone())
            .with_show_download_progress(false);

        let model = TextEmbedding::try_new(options)
            .map_err(|error| EmbedError::ModelUnavailable(error.to_string()))?;

        Ok(self.session.get_or_init(|| Mutex::new(model)))
    }
}

impl Embedder for FastembedEmbedder {
    fn model_id(&self) -> &'static str {
        MINILM_L6_V2_Q_MODEL_ID
    }

    fn dimension(&self) -> usize {
        MINILM_L6_V2_Q_DIMENSION
    }

    /// True once the model session has loaded in this process, OR the model
    /// files are already present on disk from a prior run. Never triggers a
    /// download or a model load — cheap and side-effect free, per the trait
    /// contract.
    fn is_ready(&self) -> bool {
        self.session.get().is_some() || is_model_downloaded(&self.cache_dir)
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let session = self.session()?;
        let mut model = session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // `None` batch_size defers to fastembed's own default batching.
        // `fastembed::Embedding` is a `Vec<f32>` alias, so this is already
        // the trait's `Vec<Vec<f32>>` return shape.
        model
            .embed(texts, None)
            .map_err(|error| EmbedError::Failed(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_embedder_is_deterministic_and_normalized() {
        let embedder = FakeEmbedder::new(384);
        let texts = vec!["hello".to_string(), "world".to_string()];
        let a = embedder.embed(&texts).expect("embed");
        let b = embedder.embed(&texts).expect("embed");
        assert_eq!(a, b, "same input must produce identical vectors");
        assert_ne!(a[0], a[1], "different texts must produce different vectors");
        for vector in &a {
            assert_eq!(vector.len(), 384);
            let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-5, "vector must be L2-normalized, norm={norm}");
        }
    }

    #[test]
    fn fake_embedder_empty_input_yields_empty_output() {
        let embedder = FakeEmbedder::new(8);
        assert!(embedder.embed(&[]).expect("embed").is_empty());
    }

    #[test]
    fn model_cache_dir_is_scoped_under_conclave() {
        let dir = model_cache_dir();
        assert!(dir.ends_with("Conclave/models"), "unexpected cache dir: {dir:?}");
    }

    #[test]
    fn is_model_downloaded_false_for_missing_dir() {
        let missing = std::env::temp_dir().join("conclave-embedder-test-missing-dir-xyz");
        assert!(!is_model_downloaded(&missing));
    }

    #[test]
    fn fastembed_embedder_is_ready_reflects_uninitialized_session() {
        let embedder =
            FastembedEmbedder::new(std::env::temp_dir().join("conclave-embedder-test-empty"));
        assert!(
            !embedder.is_ready(),
            "fresh embedder over an empty dir must not report ready"
        );
    }

    #[test]
    fn fastembed_embedder_reports_model_identity() {
        let embedder = FastembedEmbedder::with_default_cache_dir();
        assert_eq!(embedder.model_id(), MINILM_L6_V2_Q_MODEL_ID);
        assert_eq!(embedder.dimension(), MINILM_L6_V2_Q_DIMENSION);
    }

    #[test]
    fn fastembed_embedder_empty_input_never_touches_the_model() {
        // Must short-circuit before `session()` — an empty cache dir would
        // otherwise force a session load (and, on a cold cache, a network
        // download) just to answer a call with nothing to embed.
        let embedder =
            FastembedEmbedder::new(std::env::temp_dir().join("conclave-embedder-test-empty-2"));
        assert!(embedder.embed(&[]).expect("embed empty").is_empty());
        assert!(!embedder.is_ready(), "embedding nothing must not load the model");
    }

    #[test]
    #[ignore = "downloads the real MiniLM model on a cold cache; run manually with --ignored"]
    fn fastembed_embedder_embeds_real_text_and_becomes_ready() {
        // A dedicated cache dir, not `with_default_cache_dir()` — the shared
        // Conclave cache dir may already be warm from an earlier run (e.g.
        // the T1 spike), which would trigger the "already on disk" branch of
        // `is_ready` and make the pre-embed assertion below meaningless.
        let cache_dir = std::env::temp_dir().join("conclave-embedder-test-real-embed");
        // This dir is fixed (not per-run unique) so a warm cache from a prior
        // run of THIS test doesn't leave it behind: remove it first so the
        // pre-embed `!is_ready` assertion below stays meaningful every run.
        let _ = std::fs::remove_dir_all(&cache_dir);
        let embedder = FastembedEmbedder::new(cache_dir);
        assert!(!embedder.is_ready(), "must not report ready before the first embed call");

        let texts = vec!["workspace memory system".to_string()];
        let embeddings = embedder.embed(&texts).expect("embed");

        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0].len(), MINILM_L6_V2_Q_DIMENSION);
        let norm = embeddings[0]
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "expected a unit vector, norm={norm}");
        assert!(embedder.is_ready(), "must report ready once the session is loaded");
    }

    #[test]
    #[ignore = "downloads the real MiniLM model on a cold cache; run manually with --ignored"]
    fn fastembed_embedder_concurrent_cold_embeds_do_not_race() {
        // F1 regression test: before the init_lock fix, N threads racing into
        // a cold `embed()` would each observe `session.get() == None` and
        // each call `TextEmbedding::try_new` concurrently (double
        // download + double model load against the same cache dir). This
        // drives a real concurrent cold start and asserts every thread still
        // gets a correct, consistent result with no panic/deadlock.
        let cache_dir =
            std::env::temp_dir().join("conclave-embedder-test-concurrent-cold-start");
        let _ = std::fs::remove_dir_all(&cache_dir);
        let embedder = std::sync::Arc::new(FastembedEmbedder::new(cache_dir));

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let embedder = std::sync::Arc::clone(&embedder);
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait(); // maximize the chance all 4 hit `embed` at once
                    embedder
                        .embed(&[format!("concurrent cold start {i}")])
                        .expect("embed")
                })
            })
            .collect();

        for handle in handles {
            let embeddings = handle.join().expect("thread panicked");
            assert_eq!(embeddings.len(), 1);
            assert_eq!(embeddings[0].len(), MINILM_L6_V2_Q_DIMENSION);
        }
        assert!(embedder.is_ready());
    }
}

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
}

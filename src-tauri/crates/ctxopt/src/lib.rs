//! ctxopt — pure, deterministic context-optimization logic for the Conclave
//! agent proxy. serde_json is the only dependency; all async/IO lives in the
//! engine (spec D2/D6, plan Global Constraints).

pub const DEFAULT_HIGH_WATER: f32 = 0.70; // evaluate elisions above 70% of window
pub const RE_EVAL_GROWTH: f32 = 1.10; // re-evaluate only after +10% growth
pub const RECENT_KEEP: usize = 10; // never elide within the last 10 messages
pub const MIN_ELIDE_BYTES: usize = 600; // never elide small results
pub const LEDGER_CAP: usize = 64; // LRU conversation cap

pub const DEFAULT_CEILING_TOKENS: usize = 300_000; // C: evaluate a checkpoint above this effective-context estimate
pub const RECENT_TAIL_MSGS: usize = 15; // keep the last ~10–20 messages verbatim (never frozen)
pub const MIN_NET_SAVING_TOKENS: usize = 40_000; // M: floor on projected net token saving to proceed to sampling
pub const LOW_WATER_TOKENS: usize = 200_000; // L: projected post-checkpoint tokens must land at/below this (< ceiling)

pub mod analyze;
pub mod apply;
pub mod checkpoint;
pub mod estimate;
pub mod ledger;
pub mod policy;
pub mod request;
pub mod summary;
pub mod validate;

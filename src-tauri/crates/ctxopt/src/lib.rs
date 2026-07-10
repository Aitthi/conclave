//! ctxopt — pure, deterministic context-optimization logic for the Conclave
//! agent proxy. serde_json is the only dependency; all async/IO lives in the
//! engine (spec D2/D6, plan Global Constraints).

pub const HIGH_WATER: f32 = 0.70; // evaluate elisions above 70% of window
pub const RE_EVAL_GROWTH: f32 = 1.10; // re-evaluate only after +10% growth
pub const RECENT_KEEP: usize = 10; // never elide within the last 10 messages
pub const MIN_ELIDE_BYTES: usize = 600; // never elide small results
pub const LEDGER_CAP: usize = 64; // LRU conversation cap

pub mod estimate;

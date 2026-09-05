//! Shared vocabulary of the measured-usage collectors
//! (docs/plans/2026-09-05-usage-engine.md).
//!
//! Every collector — transcript importers, direct provider chat/fusion, the
//! one-shot draft runner — and the `usage.overview` reader agree here on the
//! source names, the provider derived from a CLI harness, and the instant the
//! in-process collectors came online. The module is deliberately free of
//! database and Tauri types so runtime code can depend on it without pulling
//! the command layer in.

use chrono::{DateTime, Utc};
use std::sync::OnceLock;

// ── Sources this build collects ──────────────────────────────────────────────

/// Claude Code transcript importer.
pub const SOURCE_CLAUDE_TRANSCRIPT: &str = "claude-code";
/// Codex transcript importer.
pub const SOURCE_CODEX_TRANSCRIPT: &str = "codex";
/// Direct provider chat (`runtime::chat`).
pub const SOURCE_DIRECT_CHAT: &str = "chat";
/// Fusion panel/judge/synthesis provider calls.
pub const SOURCE_FUSION: &str = "fusion";
/// Non-persistent one-shot draft invocations.
pub const SOURCE_DRAFT: &str = "draft";

/// The source set a `complete` coverage answer must account for.
///
/// This list is DECLARED, not derived from the coverage table: inferring the
/// required sources from whichever rows exist would let a single collector's
/// interval certify a window nobody else observed. A source with no interval
/// covering the window keeps the answer `partial`, which is the honest reading
/// of "we never heard from that collector".
pub const COLLECTED_SOURCES: &[&str] = &[
    SOURCE_CLAUDE_TRANSCRIPT,
    SOURCE_CODEX_TRANSCRIPT,
    SOURCE_DIRECT_CHAT,
    SOURCE_FUSION,
    SOURCE_DRAFT,
];

/// Collector version stamped on every event, coverage interval and cursor this
/// build writes. Bump when a parser's meaning of a stored field changes.
pub const COLLECTOR_VERSION: &str = "v1";

// ── Normalized usage ─────────────────────────────────────────────────────────

/// Token usage a source reported for ONE completed response or invocation,
/// normalized to the storage contract (`repo::model_usage`): `input_tokens`
/// includes cached input, `output_tokens` includes reasoning, the rest are
/// subsets kept as provenance. A component the source did not report — or
/// reported in a shape this build cannot verify — is `None`, never 0.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MeasuredUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_write_input_tokens: Option<i64>,
    pub reasoning_output_tokens: Option<i64>,
}

/// A non-negative integer counter, or `None` for anything else (absent,
/// negative, fractional, a string). Strictness is the point: a value this
/// function cannot vouch for is unknown, not zero.
pub fn counter(value: &serde_json::Value) -> Option<i64> {
    value.as_i64().filter(|n| *n >= 0)
}

// ── Provider identity ────────────────────────────────────────────────────────

/// The provider id behind a CLI harness, in the vocabulary of the `provider`
/// table (`"anthropic"` / `"openai"`).
///
/// A CLI agent names no provider of its own, so both a context gauge and the
/// events its transcript produces derive it from the harness — through this one
/// function, so the two always land on the same model key.
pub fn provider_for_cli_kind(cli_kind: &str) -> Option<&'static str> {
    match cli_kind {
        "claude-code" => Some("anthropic"),
        "codex" => Some("openai"),
        _ => None,
    }
}

// ── In-process collectors ────────────────────────────────────────────────────

static COLLECTORS_ONLINE_SINCE: OnceLock<DateTime<Utc>> = OnceLock::new();

/// Record that the in-process collectors (draft, chat, fusion) are live from
/// this instant. Called once at engine startup; later calls are no-ops.
///
/// These collectors observe every invocation the process makes, so the only
/// honest coverage claim they can write is "from the moment the process came
/// up until now" — a window no draft, chat or fusion call could have escaped.
/// The instant is captured here rather than at the first write so an idle
/// hour before the first draft is not reported as unobserved.
pub fn mark_collectors_online() {
    let _ = COLLECTORS_ONLINE_SINCE.set(Utc::now());
}

/// When the in-process collectors came online, or `None` if nothing marked
/// them (test state). A collector with no online instant writes no coverage:
/// it has no proof of when it started watching.
pub fn collectors_online_since() -> Option<DateTime<Utc>> {
    COLLECTORS_ONLINE_SINCE.get().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_are_non_negative_integers_or_unknown() {
        use serde_json::json;
        assert_eq!(counter(&json!(42)), Some(42));
        assert_eq!(counter(&json!(0)), Some(0));
        assert_eq!(counter(&json!(-1)), None);
        assert_eq!(counter(&json!(12.5)), None);
        assert_eq!(counter(&json!("42")), None);
        assert_eq!(counter(&json!(null)), None);
    }

    #[test]
    fn provider_follows_the_harness() {
        assert_eq!(provider_for_cli_kind("claude-code"), Some("anthropic"));
        assert_eq!(provider_for_cli_kind("codex"), Some("openai"));
        assert_eq!(provider_for_cli_kind("antigravity"), None);
        assert_eq!(provider_for_cli_kind(""), None);
    }

    #[test]
    fn every_collected_source_is_distinct() {
        let mut sorted = COLLECTED_SOURCES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), COLLECTED_SOURCES.len());
    }
}

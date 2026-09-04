//! Per-model Codex context-window table (plan `2026-07-11-codex-uplift.md`,
//! ruling R2/R3).
//!
//! Codex's own default (observed 258,400 on gpt-5.4) wastes the documented
//! max window most of these models actually serve, so the Builder's "Auto"
//! context-window setting resolves through this table instead of leaving
//! codex to pick its own default. Values are the Codex-EFFECTIVE max (not
//! always the raw API window — see `gpt-5.5` below), per Aoki's 2026-07-09
//! precedent (`docs/plans/2026-07-09-codex-context-window-actual-max.md`)
//! and the human's clarification "คือเอา max ของ model ที่ทำได้จริงๆ".

/// Look up the documented Codex-effective context window for a model id.
///
/// Matches on the trimmed model id, exact only (no prefix/family fallback —
/// an unknown id returns `None` so callers can let codex pick its own
/// default rather than guessing). `model` is normally `AgentDefinition.model`
/// (`Option<String>`), which callers pass through as `Option<&str>`.
pub fn codex_model_context_window(model: &str) -> Option<i64> {
    match model.trim() {
        // Codex-effective window reported by codex-cli 0.153.2 (`codex debug
        // models`) on 2026-09-05. This intentionally differs from any API
        // headline window: launch/compaction safety follows the runtime cap.
        "gpt-6-astra" => Some(272_000),

        // GPT-5.6 family: OpenAI's frontier-models page documents a 1.05M
        // API context window, but Codex enforces a SERVER-side ceiling well
        // below that — live-verified 2026-07-11 (`codex debug models` on
        // codex-cli 0.144.1 reports context_window=372000 for all three),
        // corroborated by github.com/openai/codex issue #31860 (open bug:
        // a 1.05M client-side override does not lift the ~372-380K
        // server-enforced cap). Ruling: task codex-models-auto-ctx challenge
        // 89599d2e, upheld by Detoro 2026-07-11 (plan R3 amended, commit
        // 217437a) — using 1_050_000 here would set the 95% auto-compact
        // limit (~997K) so high it would never fire before the real cap,
        // which is actively harmful, not just wrong. UNSTABLE value — gpt-5.6
        // shipped only 2 days before this was measured and the bug report is
        // active/upvoted; re-check at the next Codex CLI version bump.
        "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" => Some(372_000),

        // gpt-5.4 serves its full 1.05M API window in Codex.
        "gpt-5.4" => Some(1_050_000),

        // gpt-5.5's API window is 1.05M, but Codex caps it at 400K — verified
        // 2026-07-09, Codex-effective max is below the API max here.
        "gpt-5.5" => Some(400_000),

        "gpt-5.4-mini" => Some(400_000),
        "gpt-5-codex" => Some(400_000),
        "gpt-5.3-codex" => Some(400_000),

        // Spark is the small/fast variant — 128K is its actual served window.
        "gpt-5.3-codex-spark" => Some(128_000),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_models_resolve_documented_max() {
        assert_eq!(codex_model_context_window("gpt-6-astra"), Some(272_000));
        assert_eq!(codex_model_context_window("gpt-5.4"), Some(1_050_000));
        assert_eq!(codex_model_context_window("gpt-5.5"), Some(400_000));
        assert_eq!(codex_model_context_window("gpt-5.4-mini"), Some(400_000));
        assert_eq!(codex_model_context_window("gpt-5-codex"), Some(400_000));
        assert_eq!(codex_model_context_window("gpt-5.3-codex"), Some(400_000));
        assert_eq!(
            codex_model_context_window("gpt-5.3-codex-spark"),
            Some(128_000)
        );
    }

    #[test]
    fn gpt_5_6_family_resolves_codex_enforced_ceiling() {
        for id in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
            assert_eq!(codex_model_context_window(id), Some(372_000), "{id}");
        }
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(codex_model_context_window("some-future-model"), None);
        assert_eq!(codex_model_context_window(""), None);
    }

    #[test]
    fn whitespace_is_trimmed_before_matching() {
        assert_eq!(codex_model_context_window("  gpt-5.4  "), Some(1_050_000));
        assert_eq!(codex_model_context_window("\tgpt-5.5\n"), Some(400_000));
    }
}

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
        // GPT-6 family: human direction 2026-09-23 "1M". OpenAI's API window
        // is 1.05M; codex clamps any request to the codex-cli 0.155.1
        // catalogue max_context_window 872_000, so the table carries that
        // Codex-effective max. Live-verified on 0.155.1: codex reports 828,400
        // usable (Conclave's meter shows 816,400 after the 12K baseline), the
        // same runtime as the Builder's 1M choice.
        "gpt-6-astra" | "gpt-6-sol" | "gpt-6-luna" => Some(872_000),

        // GPT-5.6 family: codex-cli 0.155.1 `codex debug models` (2026-09-23)
        // reports context_window=272000 (default) / max_context_window=872000.
        // The July ~372K server-side cap is refuted by local sessions serving
        // 526K-616K input with no errors (gpt-5.6-sol 526,265 on 0.154.0,
        // 2026-09-17; gpt-5.6-luna 616,384 on 0.153.4, 2026-09-10; challenge
        // 5f6cca31). History: 372_000 was the cap measured 2026-07-11 on
        // 0.144.1 (challenge 89599d2e).
        "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" => Some(872_000),

        // gpt-5.5: codex-cli 0.155.1 `codex debug models` (2026-09-23)
        // reports context_window=272000 / max 272000; the API window is 1.05M.
        // History: 400_000 was the Codex cap verified 2026-07-09.
        "gpt-5.5" => Some(272_000),

        // gpt-5.4, gpt-5.4-mini, gpt-5-codex, gpt-5.3-codex and
        // gpt-5.3-codex-spark are absent from the codex-cli 0.155.1 catalogue
        // (2026-09-23), so these values are unchanged for lack of evidence.
        // gpt-5.4 serves its full 1.05M API window in Codex.
        "gpt-5.4" => Some(1_050_000),

        "gpt-5.4-mini" => Some(400_000),
        "gpt-5-codex" => Some(400_000),
        "gpt-5.3-codex" => Some(400_000),

        // Spark is the small/fast variant — 128K is its actual served window.
        "gpt-5.3-codex-spark" => Some(128_000),

        _ => None,
    }
}

/// Stored codex `context_window` token that pins the 1M window (plan
/// `2026-09-07-codex-context-window-1m-option.md`, ruling D1) — the same
/// literal claude-code uses for its `[1m]` model suffix.
pub const CODEX_CONTEXT_WINDOW_1M: &str = "1m";

/// Context window the 1M choice launches with (ruling D2/D3).
pub const CODEX_CONTEXT_WINDOW_1M_TOKENS: i64 = 1_000_000;

/// Auto-compact limit the 1M choice launches with (ruling D2). The human
/// chose 900_000 explicitly — it is NOT the 95 % derivation the Auto path
/// applies to table values (that would be 950_000).
pub const CODEX_CONTEXT_WINDOW_1M_AUTO_COMPACT_TOKENS: i64 = 900_000;

/// Resolve the codex context window the Builder choice actually means
/// (plan `2026-09-07-codex-context-window-1m-option.md`, ruling D3): the
/// stored `context_window = "1m"` (trimmed, exact) pins 1_000_000 for EVERY
/// model, known or unknown; absent or any other stored value is "Auto" and
/// falls through to [`codex_model_context_window`]. Legacy numerics such as
/// `"258400"` are deliberately never parsed (ruling D1).
pub fn codex_effective_context_window(model: &str, context_window: Option<&str>) -> Option<i64> {
    if context_window.map(str::trim) == Some(CODEX_CONTEXT_WINDOW_1M) {
        return Some(CODEX_CONTEXT_WINDOW_1M_TOKENS);
    }
    codex_model_context_window(model)
}

/// Codex's status-line baseline (`codex-rs/protocol/src/protocol.rs:2391`
/// `BASELINE_TOKENS`, read at tag `rust-v0.153.4`): the tokens every session
/// spends on its system prompt before the first user turn. Codex subtracts it
/// from BOTH sides of its percentage, so a meter that ignores it reads roughly
/// double on a fresh session. A future Codex may change the constant; percents
/// would then drift by well under two points.
pub const CODEX_BASELINE_TOKENS: i64 = 12_000;

/// Normalize a raw `(total_tokens, model_context_window)` pair into the pair
/// Codex's own status line divides (plan `2026-09-07-codex-meter-parity.md`,
/// ruling E1).
///
/// `percent_of_context_window_remaining` (`protocol.rs:2425-2437`) computes
/// `(window - BASELINE - max(total - BASELINE, 0)) / (window - BASELINE)`, so
/// subtracting the baseline from both sides here makes
/// `round(tokens / limit * 100)` equal `100 - remaining` exactly. Windows at or
/// below the baseline are meaningless to normalize (no real model has one) and
/// pass through unchanged so small synthetic readings keep their meaning.
pub fn codex_normalize_reading(total: i64, window: i64) -> (i64, i64) {
    if window > CODEX_BASELINE_TOKENS {
        (
            (total - CODEX_BASELINE_TOKENS).max(0),
            window - CODEX_BASELINE_TOKENS,
        )
    } else {
        (total, window)
    }
}

/// One model row of Codex's catalog cache, reduced to the two fields that
/// decide the usable window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCatalogEntry {
    /// Server-enforced ceiling a configured `model_context_window` is clamped
    /// to (`models-manager/src/model_info.rs:25-33`). `None` when the catalog
    /// omits it — treated as "no cap".
    pub max_context_window: Option<i64>,
    /// Headroom Codex keeps below the resolved window
    /// (`protocol/src/openai_models.rs:488-497`); every live entry is 95.
    pub effective_context_window_percent: i64,
}

/// Percent Codex assumes when the catalog does not say otherwise.
const CODEX_DEFAULT_EFFECTIVE_PERCENT: i64 = 95;

/// Codex's model catalog cache (`$HOME/.codex/models_cache.json`), parsed for
/// the two clamp inputs above.
///
/// The file is Codex's PRIVATE cache: its shape can change at any version
/// bump, so every load path degrades to [`CodexCatalog::empty`] (no cap, 95 %)
/// rather than failing — a wrong meter seed is recoverable at the first
/// `token_count` event; a spawn that errors out is not.
#[derive(Debug, Clone, Default)]
pub struct CodexCatalog {
    entries: std::collections::HashMap<String, CodexCatalogEntry>,
}

impl CodexCatalog {
    /// A catalog that knows no models — every lookup misses.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parse a `models_cache.json`. A missing file, unreadable bytes, invalid
    /// JSON, or an unexpected shape all yield [`CodexCatalog::empty`].
    pub fn load(path: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::empty();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Self::empty();
        };
        let Some(models) = value.get("models").and_then(serde_json::Value::as_array) else {
            return Self::empty();
        };
        let mut entries = std::collections::HashMap::new();
        for model in models {
            let Some(slug) = model.get("slug").and_then(serde_json::Value::as_str) else {
                continue;
            };
            entries.insert(
                slug.trim().to_string(),
                CodexCatalogEntry {
                    max_context_window: model
                        .get("max_context_window")
                        .and_then(serde_json::Value::as_i64),
                    effective_context_window_percent: model
                        .get("effective_context_window_percent")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(CODEX_DEFAULT_EFFECTIVE_PERCENT),
                },
            );
        }
        Self { entries }
    }

    /// Load the catalog Codex itself uses, `$HOME/.codex/models_cache.json`.
    /// No `$HOME` (or no file) is just an empty catalog.
    pub fn load_default() -> Self {
        match std::env::var_os("HOME") {
            Some(home) => Self::load(
                &std::path::Path::new(&home)
                    .join(".codex")
                    .join("models_cache.json"),
            ),
            None => Self::empty(),
        }
    }

    /// Look up a model by trimmed slug, exact match only (same discipline as
    /// [`codex_model_context_window`]).
    pub fn entry(&self, model: &str) -> Option<&CodexCatalogEntry> {
        self.entries.get(model.trim())
    }
}

/// The context window Codex will actually REPORT for this model and Builder
/// choice, normalized to the meter's units (plan
/// `2026-09-07-codex-meter-parity.md`, ruling E2).
///
/// Chain, mirroring Codex: take the requested window
/// ([`codex_effective_context_window`]), clamp it to the catalog's
/// `max_context_window`, keep `effective_context_window_percent` of that, then
/// drop the [`CODEX_BASELINE_TOKENS`]. Used only to SEED the meter — the first
/// `token_count` event replaces it with Codex's own number.
pub fn codex_usable_context_window(
    model: &str,
    context_window: Option<&str>,
    catalog: &CodexCatalog,
) -> Option<i64> {
    let requested = codex_effective_context_window(model, context_window)?;
    let entry = catalog.entry(model);
    let resolved = match entry.and_then(|e| e.max_context_window) {
        Some(cap) => requested.min(cap),
        None => requested,
    };
    let percent = entry.map_or(CODEX_DEFAULT_EFFECTIVE_PERCENT, |e| {
        e.effective_context_window_percent
    });
    let usable = i64::try_from(i128::from(resolved) * i128::from(percent) / 100).ok()?;
    Some(codex_normalize_reading(0, usable).1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_catalog(body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "conclave-codex-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models_cache.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    /// Shape copied from a live `~/.codex/models_cache.json` (codex-cli
    /// 0.153.4): the last entry deliberately omits `max_context_window`.
    /// The gpt-5.4 entry is synthetic (absent from the live catalogue since
    /// 0.155.1): its cap sits below the table value so the clamp stays tested.
    fn sample_catalog() -> std::path::PathBuf {
        write_catalog(
            r#"{
              "fetched_at": "2026-09-07T00:00:00Z",
              "models": [
                { "slug": "gpt-6-astra", "context_window": 272000,
                  "max_context_window": 872000,
                  "effective_context_window_percent": 95 },
                { "slug": "gpt-5.5", "context_window": 272000,
                  "max_context_window": 272000,
                  "effective_context_window_percent": 95 },
                { "slug": "gpt-5.4", "context_window": 272000,
                  "max_context_window": 500000,
                  "effective_context_window_percent": 95 },
                { "slug": "gpt-5.3-codex-spark", "context_window": 128000,
                  "max_context_window": 128000,
                  "effective_context_window_percent": 95 },
                { "slug": "gpt-uncapped", "context_window": 200000,
                  "effective_context_window_percent": 95 }
              ]
            }"#,
        )
    }

    #[test]
    fn normalize_reading_subtracts_codex_baseline() {
        // E-percent: Codex's status line works off `total - 12_000` over
        // `window - 12_000`, so the screenshot pair must normalize to the
        // pair that yields Codex's own 2 %.
        assert_eq!(codex_normalize_reading(31_632, 828_400), (19_632, 816_400));
        // Totals below the baseline floor at zero, never negative.
        assert_eq!(codex_normalize_reading(5_000, 828_400), (0, 816_400));
        // Windows at or below the baseline pass through untouched.
        assert_eq!(codex_normalize_reading(222, 8_000), (222, 8_000));
        assert_eq!(codex_normalize_reading(0, 12_000), (0, 12_000));
    }

    #[test]
    fn catalog_loads_entries_leniently() {
        let path = sample_catalog();
        let catalog = CodexCatalog::load(&path);
        let astra = catalog.entry("gpt-6-astra").expect("gpt-6-astra entry");
        assert_eq!(astra.max_context_window, Some(872_000));
        assert_eq!(astra.effective_context_window_percent, 95);
        // Whitespace around the queried slug is trimmed, like the table.
        assert!(catalog.entry("  gpt-5.5  ").is_some());
        // A missing `max_context_window` is None, not an error.
        let uncapped = catalog.entry("gpt-uncapped").expect("gpt-uncapped entry");
        assert_eq!(uncapped.max_context_window, None);
        assert!(catalog.entry("gpt-not-in-catalog").is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn catalog_degrades_to_empty_on_missing_or_garbage_files() {
        assert!(CodexCatalog::load(std::path::Path::new(
            "/nonexistent/conclave/models_cache.json"
        ))
        .entry("gpt-6-astra")
        .is_none());
        for body in ["{}", "not json at all", r#"{"models": 7}"#, "[]"] {
            let path = write_catalog(body);
            let catalog = CodexCatalog::load(&path);
            assert!(
                catalog.entry("gpt-6-astra").is_none(),
                "catalog body {body:?} must degrade to empty"
            );
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn usable_context_window_clamps_to_catalog_cap_and_baseline() {
        let path = sample_catalog();
        let catalog = CodexCatalog::load(&path);
        // "1m" on gpt-6-astra: min(1_000_000, 872_000) x 95 % = 828_400,
        // minus the 12_000 baseline (E-usable + E-percent).
        assert_eq!(
            codex_usable_context_window("gpt-6-astra", Some("1m"), &catalog),
            Some(816_400)
        );
        // Auto on gpt-6-astra: table 872_000 equals the catalog cap, so
        // 872_000 x 95 % - 12_000 = 816_400 — the same usable window as "1m"
        // above, because codex clamps the 1M request to that same cap.
        assert_eq!(
            codex_usable_context_window("gpt-6-astra", None, &catalog),
            Some(816_400)
        );
        // Auto on gpt-5.5: table 272_000 equals the catalog cap, so no clamp.
        assert_eq!(
            codex_usable_context_window("gpt-5.5", None, &catalog),
            Some(246_400)
        );
        // Auto on gpt-5.4: table 1_050_000 clamped to the fixture cap
        // 500_000, x 95 % = 475_000, minus the 12_000 baseline.
        assert_eq!(
            codex_usable_context_window("gpt-5.4", None, &catalog),
            Some(463_000)
        );
        // Unknown model, empty catalog: no cap, default 95 %.
        assert_eq!(
            codex_usable_context_window("some-future-model", Some("1m"), &CodexCatalog::empty()),
            Some(938_000)
        );
        // Unknown model on Auto has no requested window at all.
        assert_eq!(
            codex_usable_context_window("some-future-model", None, &CodexCatalog::empty()),
            None
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn known_models_resolve_documented_max() {
        assert_eq!(codex_model_context_window("gpt-6-astra"), Some(872_000));
        assert_eq!(codex_model_context_window("gpt-6-sol"), Some(872_000));
        assert_eq!(codex_model_context_window("gpt-6-luna"), Some(872_000));
        assert_eq!(codex_model_context_window("gpt-5.4"), Some(1_050_000));
        assert_eq!(codex_model_context_window("gpt-5.5"), Some(272_000));
        assert_eq!(codex_model_context_window("gpt-5.4-mini"), Some(400_000));
        assert_eq!(codex_model_context_window("gpt-5-codex"), Some(400_000));
        assert_eq!(codex_model_context_window("gpt-5.3-codex"), Some(400_000));
        assert_eq!(
            codex_model_context_window("gpt-5.3-codex-spark"),
            Some(128_000)
        );
    }

    #[test]
    fn gpt_5_6_family_resolves_catalogue_max() {
        for id in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
            assert_eq!(codex_model_context_window(id), Some(872_000), "{id}");
        }
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(codex_model_context_window("some-future-model"), None);
        assert_eq!(codex_model_context_window(""), None);
    }

    #[test]
    fn effective_window_pins_1m_for_any_model() {
        // Plan 2026-09-07 D3: "1m" pins 1_000_000 regardless of the table.
        assert_eq!(
            codex_effective_context_window("gpt-5.4", Some("1m")),
            Some(1_000_000)
        );
        assert_eq!(
            codex_effective_context_window("some-future-model", Some("1m")),
            Some(1_000_000)
        );
        assert_eq!(
            codex_effective_context_window("gpt-5.4", Some(" 1m ")),
            Some(1_000_000)
        );
    }

    #[test]
    fn effective_window_falls_through_to_table_for_auto_and_legacy_values() {
        assert_eq!(
            codex_effective_context_window("gpt-5.4", None),
            Some(1_050_000)
        );
        // Legacy stored numerics are Auto (D1) — never parsed.
        assert_eq!(
            codex_effective_context_window("gpt-5.4", Some("258400")),
            Some(1_050_000)
        );
        assert_eq!(
            codex_effective_context_window("some-future-model", Some("258400")),
            None
        );
    }

    #[test]
    fn whitespace_is_trimmed_before_matching() {
        assert_eq!(codex_model_context_window("  gpt-5.4  "), Some(1_050_000));
        assert_eq!(codex_model_context_window("\tgpt-5.5\n"), Some(272_000));
    }
}

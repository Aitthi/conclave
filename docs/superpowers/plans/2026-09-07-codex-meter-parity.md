# Plan: Codex context meter parity — usable window and Codex percent

Date: 2026-09-07
Owner: Detoro 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop
Base SHA: 910c09a (main, after the codex-ctx-1m-option merge)
Implementer: Dew 60ff2775-14a2-4db4-ab44-6df5bb13bf2a · Reviewer: Mellow b3a30e7b-5a9f-4d4d-a83e-768a9326632f
Escalation: design/spec conflicts → `conclave task challenge` on this task, Detoro rules.

Human report (2026-09-07, verbatim Thai): "codex mode 1m มันจะ safe สำหรับ
compact ที่ 828k context ทำให้ context meter มันนับผิด ช่วยแก้ให้หน่อย"
+ screenshot: Codex status line `Context 2% used · 828K window` while the
Conclave chip for the same pane reads `4%`.

This AMENDS plan `docs/superpowers/plans/2026-09-07-codex-context-window-1m-option.md`
ruling D3 (meter seed 1_000_000) and the 2026-07-09 decision recorded by test
`codex_token_formula_uses_reported_last_total_without_offset`
(`src-tauri/src/engine/runtime/transcript_context.rs:1759`). Launch args (D2)
are NOT touched.

## Evidence (read before challenging anything below)

Verified against the Codex source at tag `rust-v0.153.4` (the installed
`codex-cli 0.153.4`) and the live catalog cache `~/.codex/models_cache.json`.

- **E-clamp.** `codex-rs/models-manager/src/model_info.rs:25-33`
  `with_config_overrides`: a configured `model_context_window` is
  `min(configured, model.max_context_window)`. Catalog `max_context_window`
  on this machine: gpt-6-astra / gpt-5.6-sol / -terra / -luna / gpt-reserve
  = **872_000**; gpt-5.5 / gpt-5.4-mini = 272_000; gpt-5.3-codex-spark =
  128_000. Every entry has `effective_context_window_percent: 95`.
- **E-usable.** `codex-rs/protocol/src/openai_models.rs:488-497`
  `usable_context_window = resolved_context_window * effective_context_window_percent / 100`,
  and `codex-rs/core/src/session/turn_context.rs:443-445` puts exactly that
  into every `token_count` event's `info.model_context_window`. So "1m" on
  gpt-6-astra → min(1_000_000, 872_000) × 95 % = **828_400** — the number
  in the human's screenshot and in the live rollout
  `~/.codex/sessions/2026/09/07/rollout-2026-09-07T14-24-31-*.jsonl`
  (`"model_context_window":828400`). Auto on gpt-6-astra (table 272_000)
  → 258_400, also observed live.
- **E-compact.** `openai_models.rs:499-510`: auto-compact limit =
  `min(config model_auto_compact_token_limit, resolved_window * 9 / 10)`
  → 1M on gpt-6-astra compacts at min(900_000, 784_800) = **784_800**;
  `core/src/session/context_window.rs:83-109` additionally forces
  compaction when active tokens reach the usable window (828_400). The
  human's "safe for compact at 828k" is this behaviour; our 900_000 launch
  value is harmless (Codex takes the min) and stays as mandated.
- **E-percent.** `codex-rs/protocol/src/protocol.rs:2391` `BASELINE_TOKENS
  = 12000`; `:2425-2437` `percent_of_context_window_remaining(window)` =
  `round(100 × (window − 12000 − max(total − 12000, 0)) / (window − 12000))`;
  the status line prints `Context {100 − remaining}% used`
  (`codex-rs/tui/src/chatwidget/status_controls.rs:368-398`, `total` =
  `last_token_usage.total_tokens`, `window` = the event's
  `model_context_window`). Screenshot check: total 31_632, window 828_400
  → Codex 2 %; Conclave `31_632 / 828_400` → 4 %.

Conclusion: after the first `token_count` our denominator is already
Codex's usable window (we read `info.model_context_window`,
`transcript_context.rs:657-661`). Two things are wrong: (1) the percent
ignores Codex's 12_000 baseline, (2) the pre-event seed is a bare
1_000_000 that Codex never serves (roster `contextLimit`, LaneBoard,
`conclave agent list` all show it until the first event).

## Rulings (Detoro, in-loop)

- **E1 — Codex readings are Codex-normalized.** Add to
  `src-tauri/src/engine/codex_models.rs`:
  `pub const CODEX_BASELINE_TOKENS: i64 = 12_000;` and
  `pub fn codex_normalize_reading(total: i64, window: i64) -> (i64, i64)`
  returning `(max(total − 12_000, 0), window − 12_000)` when
  `window > 12_000`, else `(total, window)` unchanged. In
  `CodexAcc::ingest_line` (`transcript_context.rs:657-676`): when the event
  carries `info.model_context_window`, store `codex_normalize_reading(total,
  window)`; when it does not, store `(max(total − 12_000, 0), fallback_limit)`
  — the fallback seed is already normalized by E2, never subtract twice.
  Result: chip % = `Math.round(tokens / limit × 100)` equals the Codex
  status line exactly (same rounding). This supersedes the 2026-07-09
  "without offset" decision on the strength of E-percent; rename that test
  to `codex_token_formula_matches_codex_status_line` and assert
  `(31_632, 828_400) → tokens 19_632, limit 816_400`. Existing small-number
  tests (`222 / 8_000`, `111 / 4_000`) pass through unchanged because
  `window ≤ 12_000`. Rejected: normalizing in the UI — the percent is
  computed in ContextBars, LaneBoard, the CLI roster and the auto-snapshot
  trigger; one seam at the source keeps them all equal.
- **E2 — the seed is Codex's usable window, catalog-clamped.** Add to
  `codex_models.rs`:
  - `pub struct CodexCatalog { entries: HashMap<String, CodexCatalogEntry> }`
    with `CodexCatalogEntry { max_context_window: Option<i64>,
    effective_context_window_percent: i64 }`.
  - `impl CodexCatalog { pub fn empty() -> Self; pub fn load(path: &Path) -> Self;
    pub fn load_default() -> Self /* $HOME/.codex/models_cache.json */;
    pub fn entry(&self, model: &str) -> Option<&CodexCatalogEntry> }`.
    `load` parses `{ "models": [ { "slug", "max_context_window",
    "effective_context_window_percent", … } ] }` with `serde_json::Value`,
    leniently: a missing file, bad JSON, or a missing field yields
    `empty()` / `None` fields — never an error, never a panic. Match on the
    trimmed slug, exact.
  - `pub fn codex_usable_context_window(model: &str, context_window: Option<&str>, catalog: &CodexCatalog) -> Option<i64>`:
    `requested = codex_effective_context_window(model, context_window)?`
    (1_000_000 for "1m", else table, else None); `cap =
    catalog.entry(model).and_then(|e| e.max_context_window)`;
    `resolved = cap.map_or(requested, |c| requested.min(c))`; `pct =
    catalog.entry(model).map_or(95, |e| e.effective_context_window_percent)`;
    `usable = resolved × pct / 100` (i128 math); return
    `Some(codex_normalize_reading(0, usable).1)`, i.e. usable − 12_000
    when usable > 12_000. Worked values (catalog as on this machine):
    gpt-6-astra "1m" → 816_400; gpt-6-astra Auto → 246_400; gpt-5.5 Auto
    (table 400_000, cap 272_000) → 246_400; unknown model "1m" with an
    empty catalog → 938_000; unknown model Auto → None.
  - `resolve_session_context_limit` (`instance.rs:151`) gains a fifth
    parameter `catalog: &CodexCatalog` and calls
    `codex_usable_context_window` on the codex branch. Both call sites
    (`instance.rs:~1261` spawn and `:~1538` forwarder) pass
    `&CodexCatalog::load_default()` loaded once per call (the file is
    ~50 KB; no caching layer). Tests pass an explicit catalog built from a
    temp JSON file or `CodexCatalog::empty()`. Rename
    `resolve_session_context_limit_codex_1m_seeds_one_million` →
    `resolve_session_context_limit_codex_1m_seeds_usable_window`.
  - `append_codex_context_window_config` is UNCHANGED (still emits the
    literal 1M pair / table pair — plan D2 and R2 stand). Only the meter
    seed changes.
- **E3 — Builder hint copy.** `src/components/builder/RuntimeSection.tsx:675`
  replace `. Some models are server-capped below 1M (GPT-5.6 ≈ 372K).` with
  `. Codex clamps this to the model's server cap and keeps 5 % headroom — GPT-6 / GPT-5.6 report ≈ 828K usable; the meter shows Codex's own numbers.`
  Update the doc comment at `RuntimeSection.tsx:29` and
  `src/ipc/types.ts:70-72` only if they repeat the 372K claim. English
  copy (memory: conclave-ui-copy-english).
- **E4 — record.** Append the §Amendment block below to
  `docs/superpowers/plans/2026-09-07-codex-context-window-1m-option.md`
  (end of Rulings, do not rewrite D3's original text).

## Global constraints (every step inherits)

- Work in the lane worktree from `conclave lane start`; `pnpm install` once
  there (memory: lane worktree needs its own install).
- TDD: failing Rust test first, then code. `cargo fmt --all` before every
  commit; `cargo clippy --all-targets -- -D warnings` clean tree-wide.
- Commit with `conclave stage commit` scoped to this task's boundary.
- UI pixel gate (CLAUDE.md): `pnpm uishot builder-edit --viewport 1440x1900`
  with the codex agent + 1M selected so the E3 hint is visible; open the
  PNG with the Read tool; attach the path in the READY note. Kill any
  foreign :1420 server first (`lsof -nP -iTCP:1420 -sTCP:LISTEN`).
- Record every gate with `conclave task gate <ws> <slug> -- <cmd>`; prose
  does not count.
- Do not touch `append_codex_context_window_config`, the launch string, or
  the per-model table values.

## Steps

1. **Rust — normalize helper (TDD).** `codex_normalize_reading` + tests:
   `(31_632, 828_400) → (19_632, 816_400)`; `(5_000, 828_400) → (0, 816_400)`;
   `(222, 8_000) → (222, 8_000)`; `(0, 12_000) → (0, 12_000)`.
2. **Rust — catalog + usable window (TDD).** `CodexCatalog` (`load` from a
   temp file with the four slugs above plus one entry missing
   `max_context_window`; `load` on a missing path → empty; on `{}` → empty)
   and `codex_usable_context_window` with the five worked values in E2.
3. **Rust — transcript reading (TDD).** E1 in `CodexAcc::ingest_line`;
   rename + rewrite the 2026-07-09 test; add a no-window-field case
   (`total 20_000`, no `model_context_window`, fallback 816_400 → tokens
   8_000, limit 816_400). Run the whole `transcript_context` module.
4. **Rust — seed.** E2 wiring in `resolve_session_context_limit` and both
   call sites; update the four `resolve_session_context_limit_*` tests and
   `forwarder_codex_known_model_seeds_table_limit_not_stored_or_default`
   (`instance.rs:3345`) to pass a catalog and expect normalized values.
5. **TS — hint copy** per E3; `pnpm exec tsc --noEmit` clean.
6. **Docs** per E4.
7. **Gates** (each via `conclave task gate`):
   - `cargo test --manifest-path src-tauri/Cargo.toml codex`
   - `cargo test --manifest-path src-tauri/Cargo.toml transcript_context`
   - `cargo test --manifest-path src-tauri/Cargo.toml` (full)
   - `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
   - `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`
   - `pnpm exec tsc --noEmit`
   - `pnpm uishot builder-edit --viewport 1440x1900`
8. Move the task to `review`, post READY note with commit SHA, gate ids,
   PNG path. Detoro merges; the human relaunches a codex 1M agent and
   compares the chip with the Codex status line (live GUI e2e needs the
   human — memory: dev instance GUI e2e).

## Amendment text for 2026-09-07-codex-context-window-1m-option.md

```
- **Amendment 2026-09-07 (Detoro, plan
  docs/superpowers/plans/2026-09-07-codex-meter-parity.md).** D3's seed of
  1_000_000 is superseded: Codex clamps `model_context_window` to the
  catalog `max_context_window` (872_000 for GPT-6 / GPT-5.6) and reports
  95 % of that (828_400) as the usable window, so the meter now seeds with
  `codex_usable_context_window` (catalog-clamped, 95 %, minus Codex's
  12_000 baseline) and every codex transcript reading is normalized the
  same way, making the chip percent equal Codex's own status line. D2's
  launch pair is unchanged; Codex itself compacts at min(900_000,
  784_800) = 784_800 and hard-stops at 828_400 on those models.
```

## Risk ledger

- `models_cache.json` is Codex's private cache: its shape can change at a
  version bump. `CodexCatalog::load` must degrade to "no cap, 95 %" rather
  than fail; a unit test covers a garbage file.
- The 12_000 baseline is a Codex constant (`protocol.rs:2391`), not
  server data. If a later Codex changes it, percents drift by < 2 points;
  note the tag it was read from in the const's doc comment.
- The stored `session.context_limit` and roster `contextLimit` drop from
  1_000_000 to 816_400 for 1M agents: expected, and the point of E2. The
  `strategic-compact` skill and any agent reading `conclave agent list`
  see Codex-true numbers from now on.
- Small-window pass-through (`window ≤ 12_000`) exists only so the
  existing tiny-number tests keep meaning; no real model has such a window.

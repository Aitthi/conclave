# Plan: Codex uplift — GPT-5.6 models, Auto context window, rtk for codex

Date: 2026-07-11
Owner: Detoro 4fb2198c-e0d9-4e4b-af9e-d4e72542bace · authority: in-loop
Human mandate (2026-07-11 ~11:1x): add the new GPT-5.6 models for codex
(screenshot of OpenAI frontier-models page), make the codex context-window
setting Auto in the UI (no manual input anymore), and make codex use rtk
(https://github.com/rtk-ai/rtk) the way claude-code does.

## Rulings (Detoro, in-loop)

- **R1 — model presets.** Add `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`
  at the FRONT of `CODEX_MODELS` (`src/components/Builder.tsx:83`), keeping the
  existing entries. Model ids stay free text end-to-end — there is no Rust
  enum/catalog to extend (verified 2026-07-11; `model` is `Option<String>`
  passed to `--model`).
- **R2 — Auto context window.** The codex numeric context-window input is
  REMOVED from Builder. Backend derives the value per model from ONE new Rust
  table `codex_model_context_window(model: &str) -> Option<i64>` in a new
  module `src-tauri/src/engine/codex_models.rs`. Consumers: (a) codex launch
  args — emit `-c model_context_window=<n>` and
  `-c model_auto_compact_token_limit=<95% of n>` only when the table knows the
  model, emit NOTHING for unknown models (codex's own default wins); (b) the
  session context limit for codex agents (context meter), with the existing
  transcript-detected window still taking precedence.
  Rejected: "Auto = never emit an override, let codex decide" — codex's
  effective default (observed 258,400 on gpt-5.4) wastes the documented 1.05M
  window, which is the whole reason the override machinery exists
  (see docs/plans/2026-07-09-codex-context-window-1m-override.md).
- **R3 — table values = documented Codex-effective max**, per Aoki's
  2026-07-09 precedent and the human's earlier clarification "คือเอา max ของ
  model ที่ทำได้จริงๆ" (docs/plans/2026-07-09-codex-context-window-actual-max.md):
  `gpt-5.4` 1_050_000 · `gpt-5.5` 400_000 (Codex cap < API window) ·
  `gpt-5.4-mini` 400_000 · `gpt-5-codex` 400_000 · `gpt-5.3-codex` 400_000 ·
  `gpt-5.3-codex-spark` 128_000. GPT-5.6 family: the API page says 1.05M
  context; whether Codex serves 5.6 at the full window is UNVERIFIED — research
  task `codex-hooks-research` pins it from official OpenAI sources. Provisional
  value 1_050_000 for all three; lane M may not go READY until the memo lands
  or Detoro rules on a discrepancy.
- **R4 — stored values.** The `context_window` column stays (claude-code still
  uses `"1m"`/`"200k"`). Codex agents stop sending it and any stored codex
  value is IGNORED at launch — auto wins. No migration.
- **R5 — rtk for codex via codex hooks.** codex-cli 0.144.1 ships a stable
  `hooks` feature with PreToolUse command hooks (verified locally 2026-07-11:
  `codex features list` → `hooks stable true`; binary carries
  `pre-tool-use.command.input/output` JSON schemas, `matcher`, a
  `codex_hooks::engine::command_runner`, and claude-compatible exit-code-2
  semantics). This SUPERSEDES rtk-builtin spec Decision #2 ("Codex agents get
  nothing", docs/superpowers/specs/2026-07-09-rtk-builtin-design.md) — that
  decision was made when codex had no hook surface. Injection happens per-spawn
  via `-c` overrides; the standing policy "never write the user's
  `~/.codex/config.toml`" (sandbox_config.rs:40) HOLDS. Exact config shape and
  wire format are pinned by the research task before lane K's plan is
  finalized.
- **R6 — Builder shows "Auto", no number.** With the TS default/max maps
  deleted, the UI has no per-model window value; it shows a muted static hint
  (e.g. "Context window: Auto — derived from the model"). Rejected: keeping a
  TS mirror of the Rust table just to display the number — two sources of
  truth that will drift.

## Global constraints (every lane inherits)

- UI Pixel Gate (CLAUDE.md): any `src/` change → `pnpm uishot builder`, OPEN
  the PNG with an image reader, attach the path in the READY note, and record
  the run via `conclave task gate`.
- Fresh lane worktrees need their own `pnpm install` before pnpm gates.
- Commit via `conclave stage commit` (boundary-scoped); never raw `git add -A`.
- App UI copy is English.
- Kill foreign vite dev servers on :1420 before trusting a uishot
  (`lsof -nP -iTCP:1420 -sTCP:LISTEN`).

## Lane M — task `codex-models-auto-ctx` (implementer: Dew)

Boundary: `src/components/Builder.tsx`,
`src-tauri/src/engine/codex_models.rs` (new),
`src-tauri/src/engine/mod.rs`,
`src-tauri/src/engine/commands/instance.rs`,
`src-tauri/src/engine/repo/session.rs`.

Changes, in build order:

1. **New `src-tauri/src/engine/codex_models.rs`**: pub fn
   `codex_model_context_window(model: &str) -> Option<i64>` returning the R3
   table (trimmed exact-match on the model id; document WHY each value in a
   comment citing the 2026-07-09 actual-max plan). Unit tests: known ids, the
   three 5.6 ids, unknown id → None, whitespace trim. Register `pub mod
   codex_models;` in `engine/mod.rs`.
2. **`instance.rs`**: change `append_codex_context_window_config`
   (instance.rs:68-82) to take the MODEL (`Option<&str>`) instead of the
   stored context_window; resolve via the table; keep the 95% auto-compact
   derivation and shell quoting exactly as-is. Update the call site
   (instance.rs:732) to pass `def.model`. Update the existing tests
   (instance.rs:1582-1614) to the new signature: known model emits both `-c`
   overrides, unknown model emits nothing, stored `context_window` is ignored.
3. **Session limit**: where codex sessions resolve their pre-detection context
   limit (`default_context_limit_for`, `session.rs:56-60`; call sites
   instance.rs:858-862, 1011-1022, 1225-1231 — inspect each), consult
   `codex_model_context_window(model)` first for codex agents, then fall back
   to `default_context_limit_for(cli_kind)`. The transcript-detected window
   (`runtime::transcript_context`) keeps precedence — do not touch that
   module.
4. **`Builder.tsx`**: add the three GPT-5.6 ids to `CODEX_MODELS` (R1). Remove
   the codex numeric input block (:1324-1352) and the now-dead scaffolding —
   `CODEX_CONTEXT_WINDOW_DEFAULTS/MAX` maps and helpers (:85-121), codex
   branches of `initialContextWindow` (:128-135), derived max/validation
   (:425-431), reset logic in `selectCliKind`/`selectModelPreset` (:457-471),
   save-time validation (:479-496). Codex saves send `contextWindow:
   undefined` (:536). Claude-code's `1m`/`200k` segmented control
   (:1283-1322) is UNTOUCHED. In place of the input, a muted hint per R6.
5. Fixture scenarios (`src/fixtures/scenarios/*.ts`): only if a handler
   asserts on contextWindow for codex — inspect, do not expand boundary
   without a challenge.

Gates before READY: `cargo test` (in src-tauri, or the engine test subset),
`pnpm build`, `pnpm uishot builder` + Read the PNG. Each recorded via
`conclave task gate`.

READY additionally requires: Guetta's memo confirming (or correcting) the
GPT-5.6 Codex windows — if the memo contradicts 1_050_000, post a task note
and wait for Detoro's ruling before READY.

## Lane R — task `codex-hooks-research` (researcher: Guetta)

No code. Deliverable = ONE memo as a task note, with sources.

1. codex-cli 0.144.1 hooks **config schema**: exact TOML shape to register a
   PreToolUse command hook (table/array keys, `matcher` semantics against the
   shell/exec tool, command string vs argv, timeout), and whether it can be
   injected per-spawn via `-c key=value` (nested TOML value on one flag) —
   prove with a live minimal hook on a scratch codex session if docs are
   thin. Local leads: `codex features list` (hooks stable), the binary embeds
   JSON schemas titled `pre-tool-use.command.input` / `.output`; try
   `codex app-server generate-json-schema` to dump them; official docs at
   github.com/openai/codex (docs/config.md, docs/hooks.md if present).
2. PreToolUse **wire format** for command rewriting: input JSON fields (where
   the shell command lives), output JSON to REWRITE the command (decision
   fields; codex analogue of claude's `hookSpecificOutput.updatedInput`), and
   exit-code semantics. Diff against claude-code's format consumed by
   `conclave rtk-hook` (src-tauri/src/bin/conclave-cli.rs `run_rtk_hook`) —
   state whether one binary can serve both with a mode flag.
3. **GPT-5.6 context windows in Codex**: official OpenAI sources for
   gpt-5.6-sol / terra / luna — API window (screenshot says 1.05M) AND any
   Codex-specific cap (precedent: gpt-5.5 is 400K in Codex vs 1.05M API).
   Also note max output and reasoning-effort levels if documented.

## Lane K — task `codex-rtk-hook` (implementer: Tiësto) — plan finalized after Lane R

Skeleton (do not claim before Detoro finalizes): inject the rtk PreToolUse
hook into the codex launch branch (instance.rs:728-760) via `-c` overrides
when `rtk_enabled != false` and both bins resolve (mirror the claude gate at
instance.rs:654-671); extend or sibling `run_rtk_hook` in conclave-cli.rs for
the codex wire format; extend the Builder rtk Toggle (:1355-1361, save at
:537-538) to codex; rtk awareness preamble sentence for codex
(agentctx.rs:476-493). Boundary overlaps lane M on Builder.tsx and
instance.rs → lane K starts only after lane M merges.

## Risk ledger

- GPT-5.6 Codex cap unverified (R3) — blocking gate on lane M READY.
- `-c` hook injection may not accept nested TOML on one flag — lane R must
  prove it live; fallback is a per-instance codex config layer via
  `CODEX_HOME`/profile IF codex offers one that does not touch the user's
  config (research question, not an implementer improvisation).
- Overriding `model_context_window` above what the serving stack honors:
  transcript-detected window takes precedence at runtime, so a wrong table
  value self-corrects on the meter — but auto-compact limit would still be
  wrong; hence R3's verify-before-READY.
- `session.rs` limit resolution happens per call site (three in instance.rs)
  — missing one leaves a stale 200k meter for codex; the plan names all
  three, verify each.

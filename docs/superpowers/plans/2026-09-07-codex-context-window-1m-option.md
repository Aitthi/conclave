# Plan: Codex context window — add a 1M option beside Auto

Date: 2026-09-07
Owner: Detoro 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop
Base SHA: 2688a903a620a9eefeefbb6b54c3ad738624c738 (main)
Implementer: Dew 60ff2775-14a2-4db4-ab44-6df5bb13bf2a · Reviewer: Mellow b3a30e7b-5a9f-4d4d-a83e-768a9326632f
Escalation: design/spec conflicts → `conclave task challenge` on this task, Detoro rules.

Human mandate (2026-09-07, verbatim Thai): "เพิ่มตัวเลือก context window ให้
codex Auto / 1M ตัว 1M ให้ใส่ -c model_context_window=1000000
-c model_auto_compact_token_limit=900000 เพิ่มมา"

Translation: the Codex agent Builder gets a segmented Context-window choice
**Auto / 1M**. Auto keeps today's behaviour exactly (per-model table). 1M
launches codex with the two literal overrides
`-c model_context_window=1000000 -c model_auto_compact_token_limit=900000`.

This AMENDS plan `docs/superpowers/plans/2026-07-11-codex-uplift.md` rulings
R2 / R4 / R6 (which removed every manual codex option). Those rulings stay
in force for Auto; this plan adds ONE more value. Append the amendment block
in §Amendment to that file — do not rewrite its original text.

## Rulings (Detoro, in-loop)

- **D1 — stored value.** Codex 1M is persisted as `context_window = "1m"`,
  the SAME token claude-code already uses (`src/ipc/types.ts:72`,
  `agent_definition.rs`). Auto is persisted as absent (`undefined` from the
  Builder → NULL). Any other stored codex value (legacy numerics such as
  `"258400"`, `"200k"`) is treated as Auto — no migration, no validation
  change in `commands/agent.rs`. Rejected: a new token like `"1000000"` —
  it would re-open the numeric-override door R2 closed and needs parsing.
- **D2 — launch args.** `append_codex_context_window_config` in
  `src-tauri/src/engine/commands/instance.rs:69` gains a third parameter
  `context_window: Option<&str>`. When it is `Some("1m")` (trimmed, exact),
  emit EXACTLY
  ` -c 'model_context_window=1000000' -c 'model_auto_compact_token_limit=900000'`
  and return — for EVERY model, known or unknown to the table. The human
  chose 900_000 explicitly; do NOT derive it as 95 % (that would be 950_000).
  Any other value falls through to today's table path unchanged.
- **D3 — meter seed.** Put the resolution in ONE place: add
  `pub fn codex_effective_context_window(model: &str, context_window: Option<&str>) -> Option<i64>`
  to `src-tauri/src/engine/codex_models.rs` returning `Some(1_000_000)` for
  `"1m"`, else `codex_model_context_window(model)`. Both
  `append_codex_context_window_config` and `resolve_session_context_limit`
  (`instance.rs:132`, gains a 4th param `context_window: Option<&str>`) call
  it. Thread `def.context_window` to every `resolve_session_context_limit`
  call site: spawn (`instance.rs:~1228`, `def` in scope) and the
  `forward_session_output` forwarder (`instance.rs:~1484`, add a
  `context_window: Option<String>` parameter next to `model` and pass it
  from the spawn site). The test call at `instance.rs:~3317` passes `None`.
  Transcript-detected windows still take precedence — this only seeds.
- **D4 — Builder UI.** In `src/components/builder/RuntimeSection.tsx:631`
  replace the static "Auto — derived from the model" block with a
  `role="radiogroup"` segmented control identical in markup/classes to the
  Claude one directly above it (`:588-629`), values
  `{ value: "auto", label: "Auto" }, { value: "1m", label: "1M" }`, typed
  `CodexContextWindow = "auto" | "1m"` exported beside `ClaudeContextWindow`.
  Hint text (10.5px tertiary, mirrors the Claude hint):
  - Auto: `Derived from the model — per-model table.`
  - 1M: `Launches with ` + mono `-c model_context_window=1000000 -c model_auto_compact_token_limit=900000` + `. Some models are server-capped below 1M (GPT-5.6 ≈ 372K).`
  Copy is English (memory: conclave-ui-copy-english).
- **D5 — Builder state.** `src/components/Builder.tsx`:
  - `initialContextWindow(def)` (`:58`) becomes cli-kind aware: claude-code →
    `"1m"`/`"200k"` as today; codex → `def?.contextWindow === "1m" ? "1m" : "auto"`;
    other kinds keep the claude default.
  - `selectCliKind` (`:504-510`): add the codex mirror — switching TO codex
    with `contextWindow === "200k"` sets `"auto"`. The existing claude branch
    already maps `"auto"` → `"200k"`. `"1m"` deliberately survives a
    kind switch in both directions.
  - `handleSave` (`:527`): `contextWindowForSave` = claude as today; codex →
    `contextWindow === "1m" ? "1m" : undefined`; other kinds `undefined`.
- **D6 — types doc.** Update the doc comment at `src/ipc/types.ts:70-72` to:
  `Codex: "1m" pins -c model_context_window=1000000 / model_auto_compact_token_limit=900000; absent = Auto (per-model table).`
- **D7 — no TS mirror of the number, no fixture change required.** The 1M
  numbers live in Rust (D3) and appear in the UI only as the literal hint
  string in D4. `src/fixtures/scenarios/data.ts:278` keeps `"258400"` — it
  exercises the legacy→Auto path.

## Global constraints (every step inherits)

- Work in the lane worktree from `conclave lane start`; `pnpm install` once
  there (memory: lane worktree needs its own install).
- TDD: write the failing Rust test first, then the code. Run
  `cargo fmt --all` before every commit; `cargo clippy --all-targets -- -D warnings`
  must be clean tree-wide.
- Commit with `conclave stage commit` scoped to this task's boundary.
- UI pixel gate (CLAUDE.md): `pnpm uishot builder-edit --viewport 1440x1900`
  AND `pnpm uishot builder --viewport 1440x1900`, open the PNGs with the
  Read tool, attach the paths in the READY note. Kill any foreign :1420
  server first (`lsof -nP -iTCP:1420 -sTCP:LISTEN`).
- Record every gate with `conclave task gate <ws> <slug> -- <cmd>`; prose
  does not count.

## Steps

1. **Rust — table helper (TDD).** In `codex_models.rs` add
   `codex_effective_context_window` + tests: `("gpt-5.4", Some("1m"))` →
   `1_000_000`; `("some-future-model", Some("1m"))` → `1_000_000`;
   `("gpt-5.4", None)` → `1_050_000`; `("gpt-5.4", Some("258400"))` →
   `1_050_000`; `("gpt-5.4", Some(" 1m "))` → `1_000_000`.
2. **Rust — launch args (TDD).** Extend `append_codex_context_window_config`
   per D2. New tests: known model + `"1m"` emits exactly the two literal
   flags and NOT the table pair; unknown model + `"1m"` emits them too;
   existing three tests updated to pass `None` and keep passing.
3. **Rust — meter seed (TDD).** Extend `resolve_session_context_limit` per
   D3; test `("codex", Some("gpt-5.4"), Some(999), Some("1m"))` → 1_000_000
   and the non-codex path ignores `context_window`. Thread the new parameter
   through `forward_session_output`.
4. **TS — RuntimeSection + Builder** per D4/D5/D6. `pnpm exec tsc --noEmit`
   clean.
5. **Docs.** Append §Amendment below to
   `docs/superpowers/plans/2026-07-11-codex-uplift.md` (verbatim).
6. **Gates** (each via `conclave task gate`):
   - `cargo test --manifest-path src-tauri/Cargo.toml codex_context_window`
   - `cargo test --manifest-path src-tauri/Cargo.toml` (full)
   - `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
   - `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`
   - `pnpm exec tsc --noEmit`
   - `pnpm uishot builder-edit --viewport 1440x1900` and `pnpm uishot builder --viewport 1440x1900`
7. Move the task to `review`, post READY note with commit SHA, gate ids, PNG
   paths. Detoro merges.

## Amendment text for 2026-07-11-codex-uplift.md (append at end of Rulings)

```
- **Amendment 2026-09-07 (Detoro, human mandate, plan
  docs/superpowers/plans/2026-09-07-codex-context-window-1m-option.md).**
  R2/R4/R6 remain the AUTO behaviour. The Builder now also offers a codex
  "1M" choice, stored as `context_window = "1m"` (same token as claude-code),
  which launches with the literal pair `-c model_context_window=1000000
  -c model_auto_compact_token_limit=900000` for any model and seeds the
  meter at 1_000_000. Absent/other stored values = Auto, unchanged.
```

## Risk ledger

- GPT-5.6 family is server-capped ≈372K (R3, issue #31860): choosing 1M
  there sets auto-compact at 900K, which will never fire before the cap.
  Accepted — the human's explicit choice; the D4 hint names it.
- `forward_session_output` already has `#[allow(clippy::too_many_arguments)]`;
  adding one parameter is fine. Check every caller compiles (grep
  `forward_session_output(`).
- `"1m"` surviving a claude↔codex kind switch is intentional, not a bug.

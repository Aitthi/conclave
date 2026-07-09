# Plan: Codex context window config in Builder

Date: 2026-07-09
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop
Human request:

- "งั้นทำให้ UI ของ codex ไม่ต้องเลือกก็ได้ ให้ใส่ input เองได้เลย แต่มีค่า max ที่ map ตาม model default ไว้ให้"

## Goal

Codex agent definitions should let the user type the desired context window
directly instead of choosing a preset. The input must be bounded by a per-model
default/max map, and launching a Codex agent must pass that number to Codex with
`-c model_context_window=<tokens>`.

This task closes the configuration part of the earlier context-meter overcount
report. It must not add a magic token offset or change transcript usage formulas.

## Product/UI Decision

Conclave is a product-register tool UI. Keep the existing Builder visual
language: compact rows, native-looking inputs, restrained accent usage, no
decorative cards, no new palette, no explanatory marketing copy.

Claude Code keeps the existing `200K`/`1M` segmented control because that maps to
Claude's model id suffix behavior. Codex gets a numeric token input because Codex
supports config override `model_context_window` rather than Claude's `[1m]`
model suffix.

## Model Window Map

Implement a single frontend map for Codex model defaults/maxima. Use it to:

- choose the default numeric value when Codex is selected and no saved numeric
  value exists;
- clamp or reject values above the selected model's maximum;
- render a short helper string naming the maximum for the selected model.

Required known values:

- `gpt-5.5`: `258400`
- `gpt-5.4`: `258400`
- `gpt-5.4-mini`: `258400`
- `gpt-5.3-codex-spark`: `121600`
- `gpt-5-codex`: `258400` as a compatibility alias for existing fixtures.

Unknown/custom Codex model ids may keep a typed numeric value but should use the
largest known maximum (`258400`) as the input max unless the implementer finds a
more reliable local model catalog. Record any better evidence in a task note
before using it.

## Data Contract

`AgentDefinition.contextWindow` already stores a string in the Rust repo layer.
Broaden the TypeScript type from `"1m" | "200k"` to `string` and update comments
so it describes two harness-specific meanings:

- Claude Code: `"1m"` appends `[1m]`; `"200k"` means no suffix.
- Codex: a decimal token count passed as `-c model_context_window=<tokens>`.

Do not add a DB migration unless the implementer proves the existing column
cannot round-trip the numeric value.

## Launch Contract

In `src-tauri/src/engine/commands/instance.rs`, in the Codex launch branch only:

- if `def.context_window` parses as a positive integer, append
  `-c model_context_window=<tokens>`;
- quote the `key=value` argument using the existing shell-quote helper;
- keep the existing `developer_instructions` and sandbox `-c` overrides;
- do not pass Claude sentinel values (`"1m"`, `"200k"`) as Codex numeric
  context unless the UI deliberately converted them to a number.

Claude launch behavior must stay unchanged.

## Builder Behavior

In `src/components/Builder.tsx`:

- keep model presets as quick-fill buttons, but include `gpt-5.3-codex-spark`;
- remove the "Codex has no equivalent" assumption;
- add a Codex-only context window row using a numeric input, not a segmented
  preset picker;
- use stable dimensions so helper text and long numbers do not resize the row
  awkwardly;
- on model preset click, set the model and set/clamp the context window to that
  model's default unless the implementation can clearly preserve a user-edited
  value without surprising resets;
- on save, send `contextWindow` for Codex as the numeric string;
- validate before saving: integer, positive, and `<= max`.

Keep the existing Claude context segmented control and custom environment block
Claude-only.

## Tests

Add or update tests for:

1. Rust launch command construction:
   - a Codex definition with `context_window = "121600"` produces
     `-c 'model_context_window=121600'` or equivalent shell-safe argument;
   - Claude `context_window = "1m"` still produces the `[1m]` model suffix and
     does not produce `model_context_window`.

2. Rust save/repo round-trip if needed:
   - numeric Codex context window strings survive `agentDef.save`/list.

3. TypeScript build:
   - `pnpm build` must pass.

## UI Pixel Gate

This task touches `src/` UI. Before READY:

1. Run `pnpm uishot builder`.
2. Open and visually inspect `.shots/builder-default.png`.
3. Attach the shot path in the READY note.
4. Record the gate:
   `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 codex-context-window-config -- pnpm uishot builder`

If the implementer changes fixture empty-state behavior, also run
`pnpm uishot builder --scenario empty`.

## Gates

- `pnpm build`
- `cd src-tauri && cargo test`
- UI Pixel Gate above

## Boundary

- `PRODUCT.md`
- `docs/plans/2026-07-09-codex-context-window-config.md`
- `src/components/Builder.tsx`
- `src/ipc/types.ts`
- `src/fixtures/scenarios/data.ts`
- `src-tauri/src/engine/commands/agent.rs`
- `src-tauri/src/engine/commands/instance.rs`
- `src-tauri/src/engine/repo/agent_definition.rs`

Do not touch transcript token usage formulas in
`src-tauri/src/engine/runtime/transcript_context.rs`.

## Review Focus

Reviewer should check:

- Codex context input is user-editable numeric input with a model max, not a
  fixed selector.
- The selected/saved number is what Codex launch receives via
  `model_context_window`.
- Unknown models do not silently save values beyond the known maximum.
- Claude behavior is unchanged.
- No transcript formula or overcount fudge was introduced.
- UI Pixel Gate evidence includes the screenshot path and visual inspection.

## Risk Ledger

- Model limits may change upstream. Keep the map isolated so future updates are
  one small edit.
- Saving a numeric context window for Codex in a string field is intentional;
  adding a numeric DB column would widen the task without benefit.
- The context meter percentage still depends on transcript-reported usage over
  the configured/observed limit. This task configures the denominator; it does
  not reinterpret usage tokens.

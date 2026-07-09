# Plan: Codex 1M context-window override cap

Date: 2026-07-09
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop
Human request:

- "258400 เองหรอ 1ล้านได้ไหม"

## Goal

Let users type up to `1,000,000` tokens for Codex context window in Builder while
keeping the per-model values as defaults. The UI must communicate that the
larger value is a manual override cap, not the model default.

## Decision

Codex docs expose `model_context_window` as a config value and say its default is
auto for the model. The observed/default values in current transcript data are
still useful defaults:

- `gpt-5.5`: `258400`
- `gpt-5.4`: `258400`
- `gpt-5.4-mini`: `258400`
- `gpt-5-codex`: `258400`
- `gpt-5.3-codex-spark`: `121600`

But they should no longer be treated as the frontend input maximum. Use a
separate UI cap:

- `CODEX_CONTEXT_WINDOW_OVERRIDE_MAX = 1_000_000`

Launch behavior already accepts positive numeric `context_window` and passes it
to Codex as `-c model_context_window=<tokens>`, so this task should be frontend
only unless the implementer finds a hard type/test failure.

## UX Requirements

In `src/components/Builder.tsx`:

- Keep model preset clicks defaulting the input to the model default value.
- Validate Codex context window as positive integer and `<= 1_000_000`.
- Update helper copy so it does not say the model max is only `258,400`.
  Preferred shape: `Default 258,400 · override up to 1,000,000`.
- Keep stable row layout and existing compact Builder styling.
- Keep Claude behavior unchanged.

## Boundary

- `docs/plans/2026-07-09-codex-context-window-1m-override.md`
- `src/components/Builder.tsx`

Do not touch backend launch code unless required by a failing test; backend
already supports positive numeric overrides after `codex-context-window-config`.

## Gates

- `pnpm build`
- UI Pixel Gate: `pnpm uishot builder`, open `.shots/builder-default.png`, attach
  the path in READY, and record the gate on this task.

## Review Focus

- User can save `1000000` for Codex from Builder.
- Default per model is still the observed model default, not 1M.
- Helper copy makes override/default distinction clear.
- Claude context controls unchanged.
- No backend transcript formula changes.

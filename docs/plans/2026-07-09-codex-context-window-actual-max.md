# Plan: Codex context-window actual model max map

Date: 2026-07-09
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop
Human clarification:

- "คือเอา max ของ model ที่ทำได้จริงๆ"

## Goal

In Builder's Codex context-window input, use the actual documented model/Codex
maximum as the input max. Do not use an arbitrary 1M override cap.

## Source Evidence

Use official OpenAI sources only for the map:

- GPT-5.5 API model page: `gpt-5.5` has `1,050,000 context window`, but OpenAI's
  GPT-5.5 launch post separately states GPT-5.5 in Codex is available with a
  `400K context window`. Because this UI launches Codex, use `400000` for
  `gpt-5.5`.
- GPT-5.4 API model page: `gpt-5.4` has `1,050,000 context window`; Codex docs
  list it as a recommended Codex model and no lower Codex-specific cap was found
  in official docs. Use `1050000`.
- GPT-5.4 mini API model page: `gpt-5.4-mini` has `400,000 context window`. Use
  `400000`.
- GPT-5-Codex API model page: `gpt-5-codex` has `400,000 context window`. Use
  `400000`.
- GPT-5.3-Codex-Spark OpenAI launch post: at launch, Codex-Spark has a `128k`
  context window. Use `128000`.
- GPT-5.3-Codex API model page: `gpt-5.3-codex` has `400,000 context window`.
  Include it as an alias if the map already handles legacy model ids.

The previously observed values `258400` and `121600` are effective/default
runtime values from Codex transcript metadata, not the actual documented max.
They may remain defaults if useful, but they must not be presented as max.

## UX Requirements

In `src/components/Builder.tsx`:

- Split Codex context values into defaults and max:
  - defaults can stay at observed effective values (`258400` for 400K-class
    models, `121600` for Spark) to match current Codex auto behavior;
  - max must use the documented map above.
- On model preset click, fill the input with the default, not necessarily the
  max.
- Validate typed value as positive integer and `<= actual model max`.
- Helper copy must clearly distinguish default from max. Preferred:
  `Default 258,400 · max 400,000`.
- Unknown/custom Codex model ids should not pretend to know an actual max. Use
  the current typed/default value and show copy such as `Custom model · max not
  verified`; validation may fall back to the largest known max only if the copy
  makes that fallback explicit. Prefer a conservative `400000` fallback if this
  keeps the UX simpler.
- Claude behavior stays unchanged.

## Boundary

- `docs/plans/2026-07-09-codex-context-window-actual-max.md`
- `src/components/Builder.tsx`
- `src/fixtures/scenarios/data.ts` only if the fixture value should be raised
  from effective default to documented max or needs a model id update.

Do not touch backend launch or transcript formula code unless a hard failing
test proves it is required. The backend already passes positive numeric values
to Codex via `model_context_window`.

## Gates

- `pnpm build`
- UI Pixel Gate: `pnpm uishot builder`, open `.shots/builder-default.png`, attach
  the path in READY, and record the gate on this task.

## Review Focus

- `gpt-5.5` max is `400000`, not API `1050000`, because this is Codex launch UI.
- `gpt-5.4` max is `1050000`.
- `gpt-5.4-mini`, `gpt-5-codex`, and `gpt-5.3-codex` max are `400000`.
- `gpt-5.3-codex-spark` max is `128000`.
- Helper copy separates default from max.
- No arbitrary 1M cap remains.
- Claude and transcript formulas are unchanged.

/**
 * Shared model / colour catalogue.
 *
 * Lifted out of `Builder.tsx` (spec D8) so the AI drafter and the Builder offer
 * exactly the same choices. These arrays are mirrored byte-for-byte by the Rust
 * constants in `src-tauri/src/engine/commands/draft.rs` (`CLAUDE_MODELS`,
 * `CODEX_MODELS`, `COLOR_SWATCHES`), which the draft validator checks against —
 * changing one side without the other makes valid drafts get rejected.
 */

/** Quick-fill model presets (the user can still type any value). */
export const CLAUDE_MODELS = [
  "claude-fable-5-1",
  "claude-opus-5",
  "claude-sonnet-5",
  "claude-haiku-4-5",
  "claude-opus-4-8",
];

/** Quick-fill Codex model presets. Context window is no longer configured
 *  here — the backend derives it per model (R2/R6, `codex_model_context_window`
 *  in `src-tauri/src/engine/codex_models.rs`) and the Builder shows "Auto". */
export const CODEX_MODELS = [
  "gpt-5.6-sol",
  "gpt-5.6-terra",
  "gpt-5.6-luna",
  "gpt-5.5",
  "gpt-5.4",
  "gpt-5.4-mini",
  "gpt-5.3-codex-spark",
];

/** Preset avatar colours offered in the Builder and drafted by the model. */
export const COLOR_SWATCHES = [
  "#5e5ce6",
  "#0a84ff",
  "#d6409f",
  "#30d158",
  "#ff9f0a",
  "#0fa3a3",
  "#ff3b30",
];

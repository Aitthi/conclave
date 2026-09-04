# Antigravity AgentDefinitionInput drafter compatibility

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Keep the existing one-shot drafter tests compiling after `AgentDefinitionInput` gains nullable `effort`, without adding Antigravity to the one-shot drafter.

## Required change

- In `src-tauri/src/engine/commands/draft.rs`, add exactly `effort: None` to the exhaustive `drafter_input` test helper constructor.
- Do not change drafter eligibility, schemas, prompt behavior, CLI dispatch, or production behavior.
- Commit this path separately under this supplemental task, on the existing `antigravity-cli-backend` lane branch.

## Verification

- Run the focused drafter tests that compile/use `drafter_input` after the backend field exists.
- Run diff-scoped rustfmt and `git diff --check`.
- The parent backend lane remains responsible for the full Rust suite.

## Exact boundary

- `src-tauri/src/engine/commands/draft.rs`

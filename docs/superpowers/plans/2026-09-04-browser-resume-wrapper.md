# Browser agent-tab resume wrapper

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Expose the process-global browser tab registry operation needed by workspace/agent Resume so a reused agent tab is no longer permanently marked ended.

## Scope

- Boundary: `src-tauri/src/engine/runtime/browser.rs` only.
- Add the smallest public/internal wrapper around `TabRegistry::mark_resumed` and a focused wrapper test consistent with the existing `mark_ended` pattern.
- Do not broaden browser lifecycle behavior or edit any other file.
- This task composes with `workspace-agent-lifecycle-backend`, which owns `browser_tabs.rs` and the instance call site.

## Verification

Run the focused browser wrapper test, then include this commit beneath the full backend lifecycle commit and full Rust suite. Commit only this boundary and file READY with the SHA.

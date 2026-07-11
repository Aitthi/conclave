# In-app browser: auto-flip agent tab to `ended` on agent exit (D4b)

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace · authority: in-loop
escalation (design/spec conflicts): Detoro (owner), final ruler
status: PLAN READY — not yet claimed

## Goal

When an agent process ends, flip ITS in-app browser tab to `ended: true` (a
read-only badge; the webview stays alive so the human can still view the final
page and closes it manually). The registry method, the `ended` flag, and the
frontend "ended" chrome ALL already ship. This task ONLY wires the call-site
into agent-lifecycle exit paths — plus the one thing the registry op does not do
on its own (emit a frontend update).

Non-goal: any change to browser multiplex, tab ownership, or the frontend badge
rendering. Those are done.

## Design decisions (SETTLED — owner ruling; do not re-open)

D-1. **Which exits mark the tab ended.** Mark `ended` on TERMINAL agent exits;
     do NOT mark it on a restart (the agent continues, its tab must stay live).
     - MARK ended: real crash/self-exit (forward_session_output EOF, epoch-current),
       `stop`, `remove`.
     - DO NOT mark ended: `restart` (kill+respawn same id) — the tab is reused by
       the new generation; marking it ended would wrongly lock a live agent's tab.
     Rationale: `ended` means "the agent that owned this tab is gone." A restart
     replaces the owner in place; the tab is not orphaned.

D-2. **Crash-death is epoch-gated.** In `forward_session_output`, call
     `mark_ended` ONLY inside the existing `if runtime.unregister_epoch(&id, epoch)`
     true-branches (co-located with the `set_status(idle)` calls). A LATE EOF from
     a superseded epoch (restart reused the id) returns false there and must NOT
     mark ended — same guard that already protects the idle transition. Never add a
     new unguarded exit.

D-3. **Plain `mark_ended`, no emit — the browser surface is POLL-driven.**
     (AMENDED 2026-07-11 after Tiësto's challenge c0542f9f — original D-3 wrongly
     assumed navigate/set_active emit a browser-state event to reuse. They do NOT.)
     Verified: `runtime/browser.rs` contains ZERO `.emit`/`bus::` calls; `navigate`
     (488) and `set_active` (532) just RETURN `BrowserState`; the frontend
     `InAppBrowserView.tsx` polls `ipc.browser.status()` every 2000ms
     (setInterval at 268-270) and has no browser `useEvent` listener. So the 2s
     poll repaints the `ended` badge with the SAME mechanism/latency as every
     other browser-state change. An emit would fire into zero listeners.
     THEREFORE: call plain `browser::mark_ended(&id)` at the exit sites — no
     `mark_ended_emit`, no AppHandle threading, no new event. Drop the
     `#[allow(dead_code)]` on `browser::mark_ended` (and on `TabRegistry::mark_ended`,
     now reached through that chain). Credit: Tiësto (challenge c0542f9f).

## File boundary

- src-tauri/src/engine/commands/instance.rs   (call-sites)
- src-tauri/src/engine/runtime/browser.rs      (add `mark_ended_emit`; keep `mark_ended`)

Everything else (browser_tabs.rs registry internals, frontend) is already done and
OUT of boundary.

## Exact call-sites (verified 2026-07-11 against HEAD c3e56d1)

instance.rs — add plain `browser::mark_ended(&id)` (D-3) at each TERMINAL exit.
There are exactly TWO physical crash-death sites, not three (Tiësto verified,
challenge c0542f9f): the `if/else` block closes at **1231** and **1235 is the
function-level SHARED TAIL** that BOTH the chat `track_context` branch and the
transcript-meter loop fall through to on EOF. The no-transcript sub-case
early-returns at **1153** and never reaches 1235.

1. **No-transcript crash EOF** — `forward_session_output`, inside `if runtime
   .unregister_epoch(&instance_id, epoch)` at **1153-1164** (early-return path),
   beside the existing `set_status(idle)`.
2. **Shared crash-EOF tail (covers chat + transcript-meter)** — same function,
   `if runtime.unregister_epoch(&instance_id, epoch)` at **1235-1246**. Same shape.
   This single site covers the chat `track_context` branch — no separate hook.
3. **stop** — `stop` at **1403**, right after `set_status(idle)` at 1418
   (unregister returned true → this call won the teardown race).
4. **remove** — `remove` at **1010**, after `unregister` at 1025. (`remove`
   deletes the agent row; the tab stays as an `ended` read-only view until the
   human closes it — per D-1 and the browser.rs:463-465 doc contract.)

DO NOT touch:
- **restart** (1446) and its kill helper's `unregister` at **1535** — D-1: no mark.

## Risk ledger

- R1 (load-bearing): the epoch guard. Every crash-death `mark_ended` MUST sit
  inside an existing `unregister_epoch == true` block. Adding it before/outside the
  guard reintroduces the fast-exit / restart race the comments at 1043-1058 & 964-969
  exist to prevent. This shared forwarder runs for EVERY agent (browser or not) —
  a regression here breaks all agent idle transitions.
- R2: `mark_ended` on a non-browser agent (no tab) must be a silent no-op. Confirm
  `TabRegistry::mark_ended` tolerates an unknown agent_id (it should — it's a
  keyed lookup). Add a test asserting no panic / no state change for an id with no tab.
- R3: double-fire. A `stop` immediately followed by a late EOF: `stop` unregisters
  (bumps epoch) → the EOF's `unregister_epoch(old)` returns false → EOF site skipped.
  So only `stop` marks ended. Idempotency: `mark_ended` twice must be harmless
  (verify: second call on an already-ended tab is a no-op). Add a test.
- R4: event storm. `mark_ended_emit` emits once per terminal exit — fine. Do not
  emit inside a loop.

## Tests (add to instance.rs #[cfg(test)] + browser.rs as needed)

Native-webview behavior resists unit TDD (per the lane's own note), but the
REGISTRY + lifecycle wiring is unit-testable without a live webview:

- T1: `mark_ended` is idempotent and no-ops for an unknown/tab-less agent id
  (browser.rs unit test on the registry).
- T2: crash-death EOF path marks the owning tab ended — extend the existing
  forwarder test at **instance.rs:1891-1936** (`forward_session_output must mark
  idle`) to also assert the tab flipped to `ended` when a tab was opened for that
  id first; and assert a LATE EOF (superseded epoch) does NOT mark ended.
- T3: `stop_marks_idle` (3062) + a sibling asserting the tab is `ended` after stop.
- T4: restart does NOT mark the tab ended (extend a restart test at 2895/3023).

## Gates (all green before READY)

- `cargo test --workspace` exit=0 (include the new T1-T4).
- `cargo clippy --all-targets -- -D warnings` exit=0.
- `cargo fmt` — DIFF-SCOPED only (repo has pre-existing whole-tree rustfmt 1.9.0
  drift; only NEW/changed lines must be clean — see memory cce377b1). Wrap as
  `conclave task gate <ws> inapp-browser-ended-detection -- <cmd>`.
- No UI pixel gate: the frontend badge already shipped + was fixture-exercised;
  this task adds no new src/ UI. (If T-anything touches src/, the UI Pixel Gate
  applies — it should not.)

## Global constraints (inherited)

- Shared main checkout: commit ONLY boundary paths via `conclave stage commit`,
  then `git reset HEAD -- <paths>` to realign the shared index (memory 66cfd1cd).
- Never read the native webview URL (browser.rs contract; crash de0a632f).
- English-only in code/comments; inter-agent messages English.

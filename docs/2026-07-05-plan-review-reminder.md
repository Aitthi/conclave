# Plan: review-reminder — memory-save nudge at the implementer's real exit verb

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Task: `review-reminder` · Implementer: Dabin eecebcbe · Reviewer: Mellow (LAND, blocking)
Status: APPROVED by human 2026-07-05 ("ทำทั้งสองอันเลย" — fix 2 of 2).
BLOCKED-UNTIL: lane `stage-status-head` merges (same file). Claim after.

## Why

DB evidence (2026-07-05): of 30 agent-authored memory chunks, 20 come from
the lead + one implementer; two agents have saved zero, and three lanes
closed today with no save. Root cause: the CLI's "Boundary reached — save
what this cost" reminder prints ONLY on `task close`
(conclave-cli.rs:854-861 capture, :1006-1011 print) — and close is the
INTEGRATOR's verb (per the prose this workspace just shipped in
skill-prose-pass). Implementers exit through `task state <ws> <slug>
review` and never see a nudge. The reminder is at the wrong door.

## Task — extend the reminder trigger (conclave-cli.rs)

- The existing close-detection also fires when the invocation is
  `task state <ws> <slug> review` or `task state <ws> <slug> abandoned`
  (abandoned lanes carry the MOST valuable unsaved lessons — what failed).
  Reuse the same capture/print mechanism and reminder text; factor the
  detection into one small fn rather than duplicating the arg-sniffing.
- `task state … in_progress` (and any other state) must NOT trigger.
- Reminder prints after a SUCCESSFUL response only (as today for close).

## Tests (mod tests in conclave-cli.rs)

1. Detection fn: `close` → true (unchanged); `state … review` → true;
   `state … abandoned` → true; `state … in_progress` → false; unrelated
   verbs → false; malformed/short argv → false, no panic.
2. Existing close-reminder tests stay green.

## Boundary

`src-tauri/src/bin/conclave-cli.rs`. Nothing else.

## Gates (commit first, then gate; from src-tauri)

- `cargo test` (full) · `cargo clippy --all-targets -- -D warnings`.
- Mellow LAND (blocking): trigger matrix matches the test list, no
  duplicated arg-sniffing, reminder only on success.

## Risk ledger

- Reaches live agents after next rebuild+install (CLI binary), same batch
  as everything else today.
- This is a nudge, not a mechanism — agents can still skip saving. The
  distiller pilot (memory-distill-queue, post-rebuild) is the harvest for
  what nudges miss; do not over-build here.
- Do NOT start before `stage-status-head` merges — same file; the lane
  board records the dependency.

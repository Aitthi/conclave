# Watch Is a Push Subscription, Not a Polling Loop

owner: 1b074885-4035-46f0-a449-b77f2be610c8 · authority: in-loop

## Goal

Stop CLI lead agents—especially Codex—from spending turns and context on
`sleep`/`msg list`/`task brief` monitoring after delegating work. Conclave
already injects actionable task events and direct agent messages into the
target PTY; `task watch` is a one-time subscription registration.

## Decisions

1. Rewrite the Leadership skill's idle-time guidance so `task watch` is
   explicitly a one-time subscription, followed by ending the current turn.
2. Name the prohibited monitoring pattern explicitly: no `sleep`, repeated
   `task watch`, `msg list`, `task brief`, or `agent list` solely to check
   progress.
3. Resume only on an injected `READY`/`BLOCKED`/`ESCALATION`, actionable task
   transition, failing gate, stall alert, direct agent message, or human input.
4. Routine progress may be pulled with `task brief` only after one of those
   events or while diagnosing a reported stall. Remove wording that invites
   periodic pulling.
5. Render a dedicated, terse success response for `conclave task watch`:
   `Subscribed. Do not poll. End your turn; Conclave will inject actionable events.`
   Preserve the engine wire response and watch semantics.

## Files

- `src-tauri/skills/leadership/SKILL.md`
- `src-tauri/src/bin/conclave-cli.rs`

## Verification

- Add or update CLI unit coverage proving `task watch` selects the dedicated
  output mode and renders the exact anti-polling sentence.
- Add a source-level regression assertion in the existing CLI test module (or
  the nearest existing builtin-skill test module) proving the Leadership skill
  contains the one-time subscription/end-turn rule and the explicit polling
  prohibition.
- Record `cargo fmt --manifest-path src-tauri/Cargo.toml --check` honestly; base
  `b0fb9fb` is already red only on out-of-boundary files. The task's formatting
  acceptance gate is `rustfmt --edition 2021 --check
  src-tauri/src/bin/conclave-cli.rs`, which must be green, with no new full-gate
  finding attributable to either boundary file.
- Run the focused `conclave-cli` binary tests.
- Run the relevant builtin-skill/repository tests if the skill assertion lives
  outside the binary test target.

## Risk ledger

- Do not block legitimate one-off reads; this change is guidance and rendering,
  not a command-rate limiter.
- Do not alter watcher persistence, notification routing, or task-event fan-out.
- Do not promise notification types Conclave does not already inject.
- Existing running agents keep their launch-sidecar snapshot until refreshed or
  relaunched; new launches must receive the revised builtin skill.

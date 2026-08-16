# Ctxmeter follow-up: lost-race response + spawn-level wiring test

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

Follow-up to ctxmeter-stale-spawn-reset (merged 841e0df). Source: Mellow's
post-merge audit, challenges fd886a93 + 9005decc (both ruled ACCEPTED) and the
ordering nit. All three items are in-boundary of instance.rs.

## 1. Lost-race early return leaks the pre-reset row (fd886a93)

instance.rs:929-932 returns the copy read at :554 when `register` loses a
race; the reset at :946 never touched it, so the caller (WorkspacePane →
ContextBars seed) gets the dead generation's tokens.

Fix — RE-READ, do not fabricate: in that early return, re-fetch via
`repo::session::get_by_instance` (fall back to the local copy on error) and
serialize the fresh row. RULED against zeroing the local copy: the race can
also be lost to an ALREADY-LIVE older generation (TOCTOU past the top
`is_live` check at :561), where the row's real value is non-zero and correct;
re-reading is right in both scenarios, zeroing only in one.

## 2. Reset ordering vs fallible skill-ids write (Mellow's nit)

`set_launched_skill_ids(...)?` at :933 sits between register and the reset —
a DB error there aborts spawn with a live child and a stale meter. Move the
`reset_context_meter_for_new_generation` call to immediately after the
register guard, BEFORE set_launched_skill_ids. The reset is best-effort and
depends only on session + resolved limit; verify `context_limit` resolution
can move up with it.

## 3. Spawn-level wiring test (9005decc)

Mutation-proven gap: removing the reset call AND re-anchoring the poll
context back to `session.started_at` at the call site keeps the whole suite
green — the three lane tests exercise helpers, none drives spawn's CLI
success branch.

Fix: one test that drives `spawn` end-to-end with a fake CLI on PATH
(temp-dir shim script named `codex` or `claude`; precedent: runtime/pty.rs
`spawn_cli_streams_output` / `spawn_cli_applies_extra_env` spawn real
children, unignored). Pre-seed the session row with a stale reading
(125_000/999); after spawn returns, assert BOTH:
- the DB row reads 0 with the resolved limit (pins the reset wiring), and
- the RETURNED payload carries contextTokens 0 (pins carrier #2 end-to-end).
Anchor wiring (generation_started_at vs session.started_at) at spawn level:
pin it if the shim can cheaply drop a pre-dated fake transcript the reader
would admit under the old anchor; if not, note why and leave the helper-level
test as the anchor's pin.
Clean up the child process (mirror the pty.rs tests' teardown).

## Gates

`cargo test --manifest-path src-tauri/Cargo.toml` green via `conclave task
gate`. Confirm the new test FAILS under both call-site mutations (no-op the
reset; restore the old anchor) before calling it a pin — record that check in
a task note.

## Risk ledger

- The re-read in the lost-race return has a tiny window (loser re-reads
  before winner's reset) — accepted; the winner's `session:context` emit
  corrects any open UI. Do not add locking for this.
- Do not re-emit from the lost-race path; the winner owns the bus.
- PATH-shim tests must not depend on a real `claude`/`codex` binary.

# Context meter: cross-agent ownership leak + viewed-agent staleness

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Bug 1 — cross-agent ownership leak (human-reported, root cause confirmed)

Human report: the UI context meter sometimes shows the WORKING agent's value
while a different agent's tab is open. Long-standing; historically blamed on
the ctx proxy (it is unrelated to the proxy).

Root cause (all in `src-tauri/src/engine/runtime/transcript_context.rs`):

- `poll_claude` scans EVERY `.jsonl` in the shared per-cwd project dir
  (`~/.claude/projects/<slug>/`) — all workspace agents share one cwd, so one
  dir holds every agent's transcript — and takes `choose_newer` across files.
- Per-file ownership gate: `finalize` requires `saw_instance`, set when any
  `attachment|user|system`-type line contains the needle
  `"own agent id is <instance_id>"` (`text_declares_own_agent_id`, ~line 662;
  `claude_value_declares_owner`, ~line 668).
- The needle legitimately appears in the agent's OWN transcript via the
  SessionStart-hook line ("You are a Conclave agent, and your own agent id
  is <id>"). BUT any transcript that ECHOES another agent's launch briefing
  also declares that peer's needle: e.g. today the lead ran
  `pgrep -fl claude` / `ps eww` and the tool_result (a `user`-type line in the
  LEAD's transcript) now contains "own agent id is <id>" for FIVE peers.
  From then on the lead's transcript passes the ownership gate for those
  peers; being the busiest file it wins `choose_newer`, and an idle peer's
  meter shows the lead's tokens. This is the "meter shows the working agent"
  sighting.

Live evidence, 2026-08-16: lead transcript
`~/.claude/projects/-Users-detoro-code-codeup/8574ebfb-*.jsonl` contains the
ps dump with all five peer briefings (search "own agent id is 60ff2775").
Idle agents currently read 0 only because their pollers have not run since
respawn; the leak fires on their next active poll.

### Fix direction (implementer's judgment within this intent)

Make ownership binding immune to content echoes. Candidates, roughly in
order of preference — verify actual transcript line shapes before choosing:

1. Restrict `claude_value_declares_owner` to line types the SessionStart hook
   context actually rides (drop `user`-type tool_result lines from the
   allowed set if the hook line is `attachment`/`system` — VERIFY against a
   real transcript; the hook context may be recorded as a user-turn
   `<system-reminder>` in some CC versions, in which case match the hook's
   exact framing, e.g. require the "You are a Conclave agent" prefix in the
   same text block, not just the needle substring).
2. And/or pin the binding: once a file has declared ownership for an
   instance via a qualifying line, record `(instance_id -> file path)` and
   ignore ownership claims from other files; a genuinely new file (respawn,
   session rotation) re-binds via the qualifying line only.
3. Belt-and-braces: reject a file whose FIRST qualifying declaration arrives
   only in a tool_result context (echo shape) — optional if (1) is solid.

Regression test: a fixture transcript pair — agent A's file containing an
echoed briefing of agent B (ps-dump shape) plus B's own file — must yield
B's meter from B's file only, never A's.

## Bug 2 — viewed-agent staleness (same lane, same files)

Screenshot evidence: lead's tab showed app meter 38% (381k, stale DB row)
while the CLI's own statusline showed 46% (456.9k). ~75k tokens behind during
a long active turn.

Cause sketch: `poll_transcript_context` (instance.rs ~1412) is throttled by
`TRANSCRIPT_POLL_INTERVAL` and fires from the instance's PTY pump; during
long tool-heavy turns with sparse PTY output, polls are rare, so the meter
lags far behind the transcript. Confirm the trigger topology, then make the
meter track an ACTIVE instance's transcript reasonably closely (e.g. poll on
transcript file growth/mtime, or a periodic tick while the instance is
working — implementer's choice; keep the blocking-pool discipline documented
at the call site).

## Boundary

- `src-tauri/src/engine/runtime/transcript_context.rs` (defining: reader,
  ownership, choose_newer)
- `src-tauri/src/engine/commands/instance.rs` (poll wiring/cadence)

No `src/` UI files — the UI subscription (`useSessionContext`,
`ContextBars.tsx`) filters by sessionId correctly and needs no change; no
uishot gate applies.

NOTE: `instance.rs` currently has no other open lane (h1-generation-400's
boundary is ctx_proxy/summary/ctxopt/count_tokens only) — do not touch those
files from this lane.

## Gates before READY

- `cd src-tauri && cargo test engine::runtime::transcript_context`
- `cd src-tauri && cargo test engine::commands::instance`
- `git diff --check`
- Record via `conclave task gate` per standing protocol.

## After merge

Live verify needs rebuild+relaunch. Then: (1) open an idle agent's tab while
the lead is working — meter must show the idle agent's own small value;
(2) during a long working turn, the app meter should track the CLI statusline
within one poll interval. Contamination from historical echo lines self-clears
once binding is fixed (old echoes stop qualifying).

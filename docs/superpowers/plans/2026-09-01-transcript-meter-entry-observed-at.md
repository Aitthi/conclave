# Fix: Claude transcript meter resurrects a closed generation's usage (stale context-notify)

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop
slug: transcript-meter-entry-observed-at
boundary: src-tauri/src/engine/runtime/transcript_context.rs
escalation: Detoro (30fa04f4-e047-4241-a9ed-f452529952be) — file a `conclave task challenge`, do not improvise

## The bug (cross-workspace incident, human-directed report)

After an agent's first `conclave restart`, the `[conclave context]` nudge fires
repeatedly with a fixed stale percentage (observed: 70–71%, four firings on
2026-08-31) while the live meter (`conclave orient`, same session row) reads
25–31%. Reported by the lokal-llm workspace lead; affected agent
`6bf7e3f6-ccdc-4167-b09c-5ac524e6f535`, workspace
`f20ac1a6-e439-4671-8b21-bb62b6ab5969`. The running app (built 2026-08-16
19:08+07, contains 4ffd85c/6911f7d/dd03eac) already has the generation-reset
and ownership fixes — this is a REMAINING hole, not a stale binary.

### Evidence (already gathered — do not re-derive)

- Warnings to the affected agent (engine DB `inter_agent_message`, UTC):
  Aug 30 10:59 (71%), Aug 31 15:20 (70%), **17:16 (71%), 19:16 (71%),
  19:56 (71%)** — the bold three fired while the real meter was 25/30/31%.
- Transcript dir `~/.claude/projects/-Users-detoro-code-labs-llms-mini-llm/`:
  old-generation file `6d32c72c-…jsonl` (5.7MB) has its **last usage row at
  2026-08-31T15:23:04.140Z** (656 assistant rows, every one carries a
  top-level `"timestamp"`), but **mtime 2026-08-31T19:51:32Z** — Claude Code
  appended non-usage metadata to the CLOSED session file 4.5h later (observed
  tail: an `agent-name` line and a `system`/`bridge_status` line timestamped
  19:51:31.924Z). The stale 71% warning fired at **19:56**, the next nudge
  tick after that append. Sibling old file `88f0e229-…jsonl` shows the same
  pattern (`last-prompt` + `cost-state` metadata tail, mtime 15:22:48 —
  seconds AFTER the successor generation's anchor).

## Root cause

`ClaudeAcc::finalize` (src-tauri/src/engine/runtime/transcript_context.rs:493-513)
uses `file_modified_at(path)` as the reading's `observed_at` — for BOTH the
generation guard (`observed_at < started_at` → reject) and `choose_newer`
recency (line 366). Claude Code appends non-usage metadata (`agent-name`,
`last-prompt`, `cost-state`, `system` subtype `bridge_status`, summary lines)
to closed session files, sometimes hours after the session ended. Such an
append bumps mtime past the new generation's anchor, so the closed file's
FULL accumulated usage (~71% of the window) becomes admissible again and,
whenever the live transcript is idle (older mtime), wins `choose_newer` and
is persisted. The nudge fires the stale pct; the injected message wakes the
agent; the agent's own activity bumps the fresh transcript past the stale
mtime, so the meter falls back to the true 25–31% — which re-arms the
one-shot nudge (`CONTEXT_NUDGE_REARM_RATIO` 0.50), so the next metadata
append fires it again. That is the exact repeat-with-fixed-number pattern
reported.

The Codex path already derives `observed_at` from the ENTRY (`CodexAcc::
ingest_line`, ~line 615: reads the timestamp out of the JSON value). Only the
Claude path trusts mtime.

## The fix (single, surgical)

All in `src-tauri/src/engine/runtime/transcript_context.rs`:

1. Make `ClaudeAcc::latest_usage` carry the usage row's own timestamp
   alongside `(line_no, tokens)` — shape is implementer's choice (widen the
   tuple or introduce a tiny struct); preserve the f674c8d1 reduction
   property (later line always wins, O(1) per poll).
2. In `ClaudeAcc::ingest_line`, when folding a usage row, parse the SAME
   line's top-level `"timestamp"` (RFC3339 with `Z`, e.g.
   `2026-08-31T15:23:04.140Z`) — mirror the codex path's chrono parsing.
3. In `ClaudeAcc::finalize`, set `observed_at` to the stored usage-row
   timestamp when present; fall back to `file_modified_at(path)` ONLY when
   the winning usage row carried no parseable timestamp (legacy/unknown
   shapes — every observed live row has one). Keep the
   `observed_at < started_at` guard exactly as is: with an entry-derived
   `observed_at`, it now permanently rejects a closed generation no matter
   how often metadata appends bump the file's mtime.
4. Update the in-file doc comments that state the old semantics — at minimum
   `bump_mtime`'s doc (line ~1302: "ranks claude readings by FILE MTIME") and
   the `ScannedReading`/`choose_newer` neighborhood if they mention mtime.

Explicitly OUT of scope: `task_timer.rs` (the nudge one-shot/re-arm logic is
correct by design — the stale WRITES are the bug), `instance.rs`, the
`collect_jsonl_files` mtime pre-filter (still sound: a file whose newest
usage row is ≥ the anchor necessarily has mtime ≥ the anchor; files it
over-admits are now rejected by finalize). If you believe any of these needs
a change, escalate with a task challenge — do not edit.

## TDD sequence (tests live in the same file's `mod tests`)

Write these failing first, then implement:

- **T1 — incident regression**: transcript owned via the SessionStart-hook
  marker (copy the fixture shape used by
  `claude_owner_via_session_start_hook_attachment`, ~line 943), all usage
  rows timestamped BEFORE `started_at`; append a non-usage metadata line
  (`system`/`bridge_status`) and `bump_mtime` the file past `started_at`.
  Poll must yield NO reading for that file (today: yields the stale tokens).
  Name the test after the incident (stale nudge after restart).
- **T2 — recency is usage recency, not mtime**: two owned files; A's last
  usage row timestamped later than B's, but B given the far-future mtime via
  `bump_mtime`. `choose_newer` must pick A (today: picks B).
- **T3 — legacy fallback**: a usage row WITHOUT `"timestamp"` still yields a
  reading with `observed_at` = file mtime (current behavior preserved).
- Then sweep the EXISTING tests: several steer admissibility/choose_newer via
  `bump_mtime` alone and will now need entry timestamps set to express the
  same intent. Rewrite them behavior-equivalent. If any pinned test's INTENT
  genuinely conflicts with the new semantics, stop and escalate with the test
  name — do not silently repurpose a pinned regression.

## Gates (run via `conclave task gate <ws> <slug> -- <cmd>`, in this order)

1. `cargo test --manifest-path src-tauri/Cargo.toml --lib transcript_context`
   — all transcript_context tests green (expected count: existing suite + 3).
2. `cargo test --manifest-path src-tauri/Cargo.toml --lib` — full lib suite
   green.

## Acceptance (mirrors the reporter's)

After this fix ships in a rebuilt app: notify and orient agree after a
restart; a warning fires only when the LIVE meter crosses a threshold; no
repeated fixed-number firings. In-lane proxy: T1–T3 green plus the swept
suite. NOTE: the running engine picks this up only after the human rebuilds
and relaunches Conclave — landing the commit does not end the incident;
say so in your READY note.

## Risk ledger

- The mtime-steered existing tests WILL fail after step 3 — that is the
  sweep, not a regression. Judge each against its stated intent.
- `FileScanState.cached` short-circuit (len+mtime) stays coherent: a
  metadata append changes len → suffix rescan → non-usage lines don't touch
  `latest_usage` → recomputed reading keeps the entry-derived `observed_at`.
- The shrink/rewrite reset path (compaction) resets the acc and rescans from
  zero — timestamps re-fold; no special handling needed.
- Same-second boundaries: `choose_newer` keeps `>=` semantics; entry
  timestamps have millisecond precision, strictly finer than the mtime they
  replace.

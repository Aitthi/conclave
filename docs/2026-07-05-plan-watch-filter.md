# Plan: watch filter — wake watchers only on decision-demanding events; faster stall paging

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Task: `watch-filter` · Implementer: Tiësto fd0dec79 · Reviewer: Mellow (LAND, blocking)
Status: APPROVED by human 2026-07-05 ("Hook แค่อันที่สำคัญ ส่วนอันปกติตั้งเวลาไว้ ถ้ามันค้างค่อย auto hook ให้ lead เข้าไปเช็ค" + "30 นาที ตั้งสั้นสัก 5-10 นาทีพอ").

## Why

`task watch` currently injects EVERY event into every watcher's session. Each
injection wakes the watcher for a full turn: context re-read (cache-miss past
5 min), a reply, and permanent context-window growth that accelerates
compaction. Measured this session: ~14 lead wake-ups across 3 lanes, of which
3 demanded action. `claimed`→`in_progress` pairs arrive seconds apart; passing
gates demand nothing. The safety net for silent-but-stuck lanes already
exists (task_timer stall engine) — it just pages too slowly (30 min).

## Decisions (settled, encode exactly these)

1. **Wake (inject to watchers) ONLY these events**:
   - `challenge` — all (deadlines are attached; latency is not optional).
   - `ruling` — a manual `task rule` answers a waiting challenger, possibly
     one proceeding on a stated default; latency is not optional here
     either. (Amended at review: original list omitted it — gap found by
     Tiësto while implementing; the deadline AUTO-ruling path notifies both
     parties via task_timer's own channel and needs nothing here.)
   - `state` → `review`, `abandoned`, or `merged`.
   - `gate` with `exit != 0` (a failing gate is a decision point; a passing
     one is ledger evidence to pull later).
   - `note` whose text starts with a marker: `READY`, `BLOCKED`, or
     `ESCALATION` (exact prefix, case-sensitive, at position 0).
   Everything else — `claimed`/`in_progress` states, passing gates,
   unmarked notes, watch/unwatch — records on the ledger as today but
   injects NOTHING. No schema change: filtering happens at notify time in
   the watcher fan-out (`commands/task.rs`, `notify_watchers` call sites
   around task.rs:706-708 and every other event emitter that notifies).
2. **No per-watch configuration in v1.** The filter is the default and only
   behavior — a flag nobody sets is complexity nobody asked for. The event
   emitters must share ONE predicate fn (e.g. `fn wakes_watchers(kind,
   payload) -> bool`) so the wake list lives in exactly one place.
3. **Stall paging gets faster** (`runtime/task_timer.rs`): `STALL_MINUTES`
   30 → **10**, `STALL_ALERT_COOLDOWN_MINUTES` 60 → **30**. Human asked for
   5-10; 10 chosen because full cargo test/clippy gates legitimately run
   5-8 quiet minutes — 5 would false-page during normal builds. The stall
   page is the safety net for important-but-unmarked notes and silent
   stalls: the lead gets pulled in to CHECK, per the human's design.
4. **Marker convention is skill prose, not just code**:
   - `src-tauri/skills/implementer/SKILL.md`, in the report-at-boundaries /
     notes cluster: add — "Prefix a note that needs the lead NOW with
     `READY`, `BLOCKED`, or `ESCALATION` — only marked notes, failing
     gates, challenges, and review/abandoned/merged transitions wake
     watchers; everything else is ledger-only. If you go quiet ≥10 minutes
     holding a claim, the stall engine pages the lead to come check."
   - `src-tauri/skills/leadership/SKILL.md`: update the "stall engine
     already pages you when a claimed lane goes quiet for 30 minutes"
     sentence to 10, and add one sentence to the idle-time/watch bullet:
     watchers wake only on decision-demanding events; pull the ledger
     (`task get`) for routine progress.
   Wording within these bullets is implementer's judgment (meaning fixed,
   phrasing free) — unlike skill-prose-pass, no verbatim requirement.
5. **tool-map SKILL.md stays untouched** — still owned by lane stage-v1
   (in review). Its watch/note rows inherit this behavior change silently;
   a one-line doc catch-up rides the NEXT tool-map touch (recorded Low,
   same bucket as the commit-first-then-gate row).

## Tests

1. Predicate unit tests, one per kind: claimed/in_progress/watch/unwatch →
   false; challenge → true; state review/abandoned/merged → true, other
   states false; gate exit 0 → false, exit ≠0 → true; note "READY …" /
   "BLOCKED …" / "ESCALATION …" → true; note "ready…" (lowercase), note
   with leading space, unmarked note → false.
2. Fan-out test: an in_progress state event on a watched task produces no
   injected message; a READY note produces exactly the same injection as
   today (shape unchanged — only the filter is new).
3. task_timer tests asserting 30/60 updated to 10/30 (find by the failing
   assertions after the constant change; do not weaken what they prove).
4. Existing skill-parse tests stay green (prose edits).

## Boundary

`src-tauri/src/engine/commands/task.rs`,
`src-tauri/src/engine/runtime/task_timer.rs`,
`src-tauri/skills/implementer/SKILL.md`,
`src-tauri/skills/leadership/SKILL.md`. Nothing else. (No new router route
→ router.rs/commands/cli.rs exempt, per memory df65b613.)

## Gates (commit first, then gate; from src-tauri)

- `cargo test` (full) · `cargo clippy --all-targets -- -D warnings`.
- Mellow LAND (blocking): predicate completeness vs decision 1 (esp. that
  challenge deadline auto-rulings still notify), single-predicate rule
  (no duplicated wake lists), stall constants + their prose mentions agree
  (10/30 everywhere), marker matching is prefix-exact.

## Risk ledger

- Reaches live agents only after next rebuild+install. Until then watchers
  keep getting everything — no compat issue, old and new coexist.
- The challenge-deadline auto-rule path (task_timer check_challenge_deadlines)
  must still notify BOTH parties — it is a `challenge`-class event; verify
  it does not route through a filtered-out kind.
- If some notes today rely on waking the lead without markers (e.g. a
  boundary-gap note like Tiësto's), the stall engine at 10 min is the
  intended catch — accept the latency; do NOT widen the wake list to
  compensate, that reopens the noise this plan closes.
- Marker prefixes are case-sensitive by decision; agents write them from
  skill prose, which this lane also ships — drift risk is low and the
  stall net covers the miss.

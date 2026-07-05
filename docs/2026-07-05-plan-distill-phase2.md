# Plan: distill-phase2 — auto-trigger for the memory distiller + F1 TOCTOU fix

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Status: APPROVED by human 2026-07-05 (pilot gate passed: combined approve
rate 4/5 = 80%, bar was 50%; recorded on tasks distiller-pilot-run1/-run2).
Tasks: `f1-toctou-fix` (Lane A, Tiësto) · `distill-auto-nudge` (Lane B, Dew)
· Reviewer: Mellow (LAND, blocking, both lanes).

## Why

The pilot proved the distiller's precision (80%) but v1 is on-demand: the lead
must remember to cut a run, which reproduces the exact failure the distiller
exists to fix (memory capture depending on someone's discipline). Phase 2 makes
the engine nudge a designated distiller agent on a slow cadence, and closes the
one known engine defect (F1) before any concurrent-reviewer future makes it
reachable.

Parent design: docs/2026-07-05-plan-memory-distill-queue.md (v1, APPROVED).
This plan amends its risk-ledger Phase 2 sketch in two places, noted inline.

## Decisions (settled by lead, encode exactly these)

1. **F1 fix is transactional, not claim-before-embed.** The v1 risk ledger
   sketched "claim state='approved' BEFORE embed"; that trades the orphan-chunk
   leak for a stuck-approved-no-chunk state whenever the embed fails after the
   claim, and needs compensation logic. AMENDED to: embed FIRST (pure, no side
   effects), then ONE sqlite transaction { upsert_chunk → set_reviewed WHERE
   state='pending'; if 0 rows updated → ROLLBACK and return the existing
   "no longer pending" error }. The approve/reject race serializes at the DB:
   the loser gets a clean error and NO chunk survives. Cost on race loss: one
   wasted embed (~ms, negligible).
2. **Auto-trigger rides the existing task_timer tick** (runtime/task_timer.rs,
   TICK_INTERVAL 5 min), NOT a task-close hook. Closes cluster (batch merges
   would fire N nudges) and a hook adds latency to the close path; the timer
   batches naturally and already owns cadence work (stalls, challenge
   deadlines).
3. **Config = one bb key per workspace, absent = OFF (kill switch).**
   Key `config:distill-auto`, value JSON:
   `{"distiller": "<agentId>", "reviewer": "<agentId>", "cooldownHours": 6}`.
   Lead sets/removes it with plain `conclave bb set/delete` — no new engine
   config surface, and deleting the key stops all future nudges instantly.
   Malformed JSON or unknown agent ids → skip and log, never crash the tick.
4. **Nudge condition** (all three, per workspace, checked on tick):
   (a) config key present and valid;
   (b) `now - note:distill-hwm > cooldownHours` (hwm is the DB-persisted
       cadence record — the skill already advances it every run; absent hwm →
       treat as satisfied);
   (c) at least one task event newer than hwm (activity signal — an idle
       workspace never gets nudged).
   Plus an IN-MEMORY per-workspace re-nudge cooldown of 60 min (Ticker
   pattern, task_timer.rs:57) so a running-but-not-yet-finished distiller
   isn't re-paged every 5-min tick. Restart resets the in-memory part only;
   worst case is one early re-nudge — acceptable, same tradeoff the stall
   engine already made.
5. **Nudge attribution follows the recorded ruling** (ADR 0008 Lane B,
   memory f51a980f): injected via commands::message::inject FROM the
   configured reviewer's instance id TO the distiller agent. No synthetic
   'system' sender. Message text names the skill, the workspace id, and tells
   the distiller to report the run summary back to the reviewer.
6. **The skill's on-demand contract is amended, not broken**: an engine nudge
   IS the reviewer asking (it is sent from the reviewer's id). SKILL.md
   updated to say so; the pilot guardrail paragraph gains the kill switch
   (delete `config:distill-auto`) and the standing rule that collapsing
   precision → turn it off first, rethink second.

## Lane A — f1-toctou-fix (engine)

Boundary: `src-tauri/src/engine/commands/memory.rs`,
`src-tauri/src/engine/repo/memory.rs`,
`src-tauri/src/engine/repo/memory_proposal.rs`.

- Rework `approve_with_embedder` (commands/memory.rs:1223): keep all
  precondition checks and the embed as-is; wrap `upsert_chunk` (:1253) +
  `set_reviewed` (:1266) in one transaction per decision 1. `set_reviewed`
  already guards `WHERE state='pending'` and returns Option — 0 rows inside
  the txn → rollback + the same error text as today (:1277).
- Repo plumbing: `upsert_chunk` and `set_reviewed` need to run on a
  transaction, not just the pool — generalize their executor (sqlx generic
  executor param or `&mut SqliteConnection` variants), smallest diff wins.
  Do NOT change their SQL.
- Tests (beside the existing mod tests, commands/memory.rs:1444+):
  (1) invariant test at repo level — upsert_chunk inside a txn that then
      fails set_reviewed rolls back: no `distilled` chunk row survives;
  (2) reject-then-approve still errors cleanly (exists today — keep green);
  (3) the happy path (:1538 area) unchanged.
- The v1 leak was found by Mellow at LAND review (credited); reachability was
  near-zero with a single reviewer — this closes it before Phase 2's nudged
  cadence ever tempts a second reviewer.

## Lane B — distill-auto-nudge (timer + skill)

Boundary: `src-tauri/src/engine/runtime/task_timer.rs`,
`src-tauri/skills/memory-distiller/SKILL.md`,
`src-tauri/skills/tool-map/SKILL.md` (config-key row).

- New `check_distill_nudge(state, now, ticker)` called from `tick`
  (task_timer.rs:90), sibling of `check_stalls` (:106) and
  `check_challenge_deadlines` (:149). Implements decisions 3–5. Reads the two
  bb keys via the existing repo layer; queries max(task_event.created_at) for
  condition (c).
- Constants beside the existing ones (:40-53): `DISTILL_RENUDGE_COOLDOWN_MINUTES
  = 60`; default `cooldownHours` when the JSON omits it = 6.
- SKILL.md edits per decision 6 (both files listed in the boundary).
- Tests (mirror the existing task_timer mod tests): no config → no nudge;
  fresh hwm (within cooldown) → no nudge; stale hwm + no task events since →
  no nudge; stale hwm + activity → exactly one inject from reviewer to
  distiller; in-memory cooldown suppresses the next tick; malformed config
  JSON → skipped, tick survives; per-workspace independence.

## Sequencing & review

- Lanes A and B are file-disjoint and independent — run in parallel,
  one implementer each. Mellow LAND review, blocking, on both (focus: A =
  transaction correctness + no behavior change on happy path; B = tick-path
  robustness — a bad config must never take down the timer that also runs
  stall alerts and challenge deadlines).
- Gates per lane (commit first, then gate; from src-tauri):
  `cargo test` (full) · `cargo clippy --all-targets -- -D warnings`.
- Integration: lead merges, re-gates on main, then next rebuild (r11) makes it
  live. First live nudge cycle gets watched end-to-end (nudge → Marty run →
  queue → review) before the lead stops babysitting it.

## Rollout config (after r11, lead action)

`conclave bb set <ws> config:distill-auto
'{"distiller": "5524d1c5-50c0-47d3-bfbd-7bb45f9d38ef", "reviewer": "bfb737ff-486d-4581-b407-95711d5e07ab", "cooldownHours": 6}'`
(Marty distills, Detoro reviews — pilot roles carried forward.)

## Risk ledger

- Tick-path blast radius: check_distill_nudge shares the tick with stall
  alerts and challenge deadlines — every failure mode must degrade to
  "skip this check", never panic or early-return out of tick. (Mellow's B
  focus.)
- hwm is skill-owned: if a distiller run dies before step 5, hwm stays stale
  and the engine re-nudges after the cooldown — self-healing, but a repeatedly
  dying distiller means repeated nudges; the reviewer will notice the silence
  and the kill switch exists.
- Reviewer availability: nudges fire regardless of whether the reviewer has
  queue time; the queue just accumulates pending rows — harmless, reviewed on
  the reviewer's cadence.
- Nothing here reaches live agents until rebuild r11.
- Deferred, unchanged from v1: codex-format transcripts (~/.codex/sessions/),
  multi-project-dir widening.

# ADR 0008: Agent Work System v1 — first-class tasks, gate ledger, watch/notify, challenges, lane manager

Date: 2026-07-04 · Status: ACCEPTED (human green-light "ไฟเขียวทำทั้งหมดเลย"; design settled in-loop, owner Detoro bfb737ff)

## Context

All multi-agent coordination today rides free-text blackboard conventions
(`plan:` / `claim:` / `progress:` keys) plus prose protocol in skill sidecars.
Every real failure this week traces to a convention the machine did not
enforce: a plain `git commit` swept peers' staged work (bb
`anomaly:b9ab709-mixed-commit`), gate evidence in the shared tree went stale
minutes after it was reported, and the lead polled progress keys by hand.

## Decision

Build five systems, in one program, on top of the existing engine
(`src-tauri/src/engine/`, sqlx + chain-builder per ADR precedent):

1. **Task objects** (Lane A): new tables `task`, `task_event` (append-only),
   `task_watch` (migration `0012_task_system.sql`), repo `engine/repo/task.rs`,
   UDS commands `task.*` in `engine/commands/task.rs`, CLI verbs
   `conclave task create|list|get|claim|state|note|gate|challenge|rule|close|watch|unwatch`.
   States: `planned → claimed → in_progress → review → merged` (+`abandoned`).
   Fields carry what the Leadership protocol demands: owner, implementer,
   file boundary (JSON pathspecs), design canon, plan body.
2. **Gate ledger** (Lane A): `conclave task gate <ws> <slug> -- <cmd...>`
   RUNS the command CLI-side, records exit code + `git rev-parse HEAD` +
   output tail as a `task_event(kind='gate')`. Evidence is recorded by the
   system at run time, not self-reported after the fact.
3. **Watch/notify + stall/deadline engine** (Lane B): `task_watch`
   subscribers get a one-line notification injected into their session via
   the existing `message.inject` path when a watched task changes. A 5-min
   engine timer flags stalls (claimed/in_progress task with no event for
   30 min and idle implementer → alert owner) and fires challenge defaults
   (deadline passed with no ruling → record `ruling{by:"default"}`, notify
   both parties).
4. **Lane manager + commit guard** (Lane C): `conclave lane start|finish`
   wraps the proven worktree lifecycle (`git worktree add -b lane/<slug>
   .claude/worktrees/<slug> main`, teardown after merge).
   `conclave lane guard install` writes a pre-commit hook that, in the
   SHARED checkout only (skip when `--git-dir` ≠ `--git-common-dir`, i.e.
   lane worktrees), rejects commits whose staged paths fall outside
   `$CONCLAVE_COMMIT_SCOPE` — making the b9ab709 accident class impossible.
5. **Lane board UI + telemetry strip** (Lane D): `LaneBoard.tsx` renders
   tasks by state with gate/challenge badges; a workspace-level context
   strip aggregates the existing per-session `session:context` events.
   New bus event `task:changed` (constant in `engine/bus.rs`, mirrored in
   `src/ipc/events.ts`). Design canon by Arta before implementation.

Memory auto-suggest rides Lane A: `task close` prints a reminder to run
`conclave memory remember` with the "what did this cost to learn" prompt.

## Alternatives rejected

- **Keep bb conventions, improve the skill prose** — rejected: prose already
  says everything; the failures happened anyway. Discipline that isn't
  enforced by structure decays at every context clear.
- **Tasks as reserved bb keys with JSON values** — rejected: no state
  machine, no append-only history, no place to hang gate runs or watches;
  every reader re-parses free text.
- **Push notifications via a new agent-facing socket protocol** — rejected:
  agents are PTY sessions; the only channel that reaches a running agent is
  stdin injection, which `message.inject` already implements. Reuse it.
- **Commit guard as server-side check** — rejected: commits happen in git,
  not through the engine; only a git hook sits at the actual choke point.

## Consequences

- The Leadership/Collaboration skill protocol shrinks to CLI verbs; bb keys
  remain for ad-hoc facts (conventions, anomalies, notes).
- `task_event` becomes the substrate any future system (metrics, audit,
  review queue v2) appends to without new schema.
- Gate evidence is attributable and timestamped, so shared-tree "all green"
  claims can be checked against the SHA they were produced at.
- Plan: `docs/2026-07-04-plan-agent-work-system.md` (lane map, frozen
  interfaces, risk ledger).

# ADR 0010: Coordinator layer — a routing hub between the Lead and the workers, and Implementer roles split by task complexity

Date: 2026-10-07 · Status: ACCEPTED (human request 2026-10-07, design doc pasted in the Lead's terminal; decisions settled in-loop, owner Detoro 30fa04f4)

## Context

Two flows have been tried in this workspace. Peer-to-peer chat looped and burned
context (closed by the Comms Protocol skill: every message goes through the
lead). The lead-centred flow stopped the loops but made the Lead — the slowest,
most expensive model on the roster (Fable 5.1) — the bottleneck: it receives every
READY note, dispatches every task, and routes every review verdict, none of which
needs its judgment.

Engine facts that constrain the fix (verified 2026-10-07 against main 97814e2):

- Task event fan-out goes to **watchers only**; the owner is not auto-subscribed
  (`engine/commands/task.rs:1593-1611`, test `task.rs:1832`). Only
  `READY`/`BLOCKED`/`ESCALATION` notes, failing gates, challenges, rulings and
  `review`/`abandoned`/`merged` transitions wake watchers (`wakes_watchers`,
  `task.rs:1545-1571`).
- A `task challenge` is additionally delivered to the **lowest common supervisor
  of challenger and owner** (`challenge_expected_ruler`, `task.rs:1657-1674`).
- Stall alerts page the **implementer's supervisor** (`task_timer.rs:310-316`).
- Ownership is set at `task create` and cannot be transferred (`router.rs:96-109`).
- Builtin roles and skills are folders shipped as Tauri resources and read at
  launch (`repo/role.rs:279-296`, `repo/skill.rs:403-420`); role descriptions are
  baked into the preamble at spawn (`instance.rs:1006-1049`).

## Decision

1. **A new builtin role `coordinator`** (display "Coordinator") with a new
   optional builtin skill `coordinator`. The Coordinator is a routing hub: it
   watches every task, dispatches claimable tasks to idle workers, forwards
   review verdicts, bounces failed reviews back (two rounds max), keeps the
   board current, and escalates to the Lead. It never rules, never edits the
   plan, never answers a design question, never implements.
2. **The Lead stays the task owner; the Coordinator is the workers'
   supervisor and a watcher on every task.** The Lead creates tasks with
   `--watchers <coordinatorId>`; `conclave position set` links every worker to
   the Coordinator and the Coordinator to the Lead. Consequences in the engine
   as it is: challenges (decisions) route to the Lead directly; stall alerts,
   review-ready transitions and marked notes land on the Coordinator; the Lead
   receives nothing routine. No engine change.
3. **Implementer roles split by task complexity:** `implementer-complex`
   ("Implementer (Complex)") and `implementer-routine` ("Implementer (Routine)")
   ship alongside the untouched generic `implementer`. All three bundle the
   `implementer` skill; they differ in the description's task class and in what
   the agent does when a task mismatches its tier. Plans tag each task
   `tier: complex|routine` so the Coordinator dispatches without asking.
4. **Routing language in the skill layer becomes supervisor-relative.** The
   Comms Protocol rule "every message goes through the lead" becomes "through
   your supervisor (`supervisorName` in `conclave agent list`; the lead when
   none)". The Implementer skill points decisions at the task owner and
   everything else at the supervisor. The Lead remains the only agent that
   writes to the human.
5. **Reporting is structured, not narrated.** Worker boundary notes open with
   the wake word and carry `task:` / `status:` / `files:` / `note:` lines. Review
   verdicts are the exact phrases `READY REVIEW-PASS @<sha>` and
   `BLOCKED REVIEW-FAIL @<sha>`; the Coordinator's merge hand-off to the Lead is
   `MERGE-READY <slug> @<sha>`.
6. **The board is the task ledger plus one blackboard key** (`coord:board`),
   not a separate status file. The Laneboard view and `conclave task list`
   already render live state; a second file would be a second source of truth.
7. **Live roster for this workspace** (applied after the rebuild, by the Lead):
   new agent **Alesso** = Coordinator on `claude-sonnet-5-5`, supervisor Detoro;
   Dew and Tiësto = Implementer (Complex) on `claude-opus-5-5`; new agent
   **Zedd** = Implementer (Routine) on `claude-sonnet-5-5`; Arta, Mellow,
   Guetta keep their roles and models; all six report to Alesso.

## Rejected alternatives

- **Coordinator owns the tasks.** Would route every challenge to an agent that
  must not rule, and ownership cannot be transferred, so the Lead could never
  take a task back. Rejected.
- **Custom role + custom skill in the DB, no rebuild.** The mandatory Comms
  Protocol skill still says "through the lead"; a DB role cannot override a
  shipped mandatory skill, so workers would follow the old rule. Rejected.
- **Calling the role "PM" / "Project Manager".** The name invites the model to
  re-prioritise and edit the plan. "Coordinator" keeps it inside its box.
- **Coordinator merges lanes.** Integration stays with the Lead (Leadership
  skill); the Coordinator hands over `MERGE-READY` and the Lead merges without a
  second review.
- **A separate status file.** See decision 6.
- **Haiku 4.5 utility agent** (lint/format/log summaries) — deferred; nothing in
  the current flow needs it. Flipping Tiësto to Sonnet 5.5 — deferred to the
  human; the role split makes the flip a one-field edit later.

## Consequences

- Workers' messages go to Alesso; the Lead sees challenges, `ESCALATION` notes,
  `MERGE-READY` hand-offs and milestone reports only.
- Plans must list tasks with slug, tier, deps and acceptance criteria, or the
  Coordinator escalates on every dispatch and the Lead is the bottleneck again
  (Leadership skill, new section "Running through a Coordinator").
- Needs app rebuild + relaunch for the new resources (roles/, skills/) to load.
- The codex chain (Aoki → Dabin/Hardwell/Armin/Marty, offline) is untouched; the
  supervisor-relative wording resolves to Aoki for them.

## Records

- Plan: `docs/plans/2026-10-07-coordinator-layer.md`
- Task: `coordinator-layer` (owner Detoro, implementer Dew, reviewer Mellow)
- Blackboard: `protocol:coordinator-flow`, `coord:board`
- Related: ADR 0005 (role system), ADR 0008 (task system),
  `docs/2026-07-05-spec-position-system.md` (routing by supervisor)

## Amendment 1 (Detoro, 2026-10-07, post-rollout — human observation in OmniChat)

The human saw workers in the OmniChat workspace "reporting to two leads". The
lines were engine watcher notifications (`[task <slug>] <actor>: note/state/gate
— …`), not worker behaviour: `task create --watchers <id>` subscribes the
**owner plus** the listed agents (`engine/commands/task.rs:563-567`, council-v1
chair semantics, spec `2026-07-10-lead-council-v1-design.md` "Command Changes"),
so every task the Lead created per decision 2 carried both the Lead and the
Coordinator in `task_watch`, and every wake fanned out to both. The Context
bullet "the owner is not auto-subscribed" is true only for the flag-less create.

Corrections:

- Context bullet 1 reads: fan-out goes to watchers only; a flag-less create
  subscribes nobody; `--watchers` subscribes the owner too (chair semantics).
  Additionally, the `review` transition pings the owner even when not watching
  when the owner has a supervisor link (`notify_expected_ruler`, spec §3.4) —
  one line per task, deduped against the watch list, kept.
- Decision 2 reads: the Lead creates tasks with
  `--watchers <coordinatorId> --no-self-watch` (new switch, task
  `task-create-no-self-watch`, plan `docs/plans/2026-10-07-task-create-no-self-watch.md`).
  Interim rule until that build is installed: `conclave task unwatch <ws> <slug>`
  right after the create.
- Decision 7 as rolled out by the human (their call, not the plan's names): the
  Coordinator is **Aitthi** (`claude-sonnet-5-5`, senior) in codeup
  (`7d560bae-09dc-4535-befb-feed5a95aa05`) and OmniChat (`bf2fdd0f-…`); Tiësto is
  Implementer (Routine) on `claude-sonnet-5-5`; Zedd is Implementer (Routine)
  junior; Dew is Implementer (Complex) on Opus. No agent named Alesso exists.

## Amendment 2 (2026-10-08) — trivial tier and the Runner role

Human request 2026-10-08: the small chores (lint/format, test runs, log
summaries, gate checks) should stop spending Opus/Sonnet tokens.

- Third dispatch tier `trivial` below `routine` → builtin role `runner`
  (bundled skill `runner`, compact; not the 183-line `implementer` skill).
- A Runner takes: lint, format, test/build runs, gate re-runs, log and
  transcript summaries. It reports exit code + log path + a five-line summary.
- A Runner never: edits logic, picks between designs, widens a task, or takes a
  routine/complex task. A defect found is a task note with evidence.
- Dispatch (Coordinator skill): a trivial task goes to an idle Runner; a Routine
  implementer may take it when no Runner is idle; never the reverse.
- The model lives on the agent definition, not the role: create the definition
  on role `runner` with `model: claude-haiku-5-5` after the rebuild.
- Plan: `docs/plans/2026-10-08-haiku-runner-orgchart-model.md` (task
  `role-runner`).

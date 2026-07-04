# Plan: builtin skills teach the task system (closes ADR 0008 adoption gap)

Date: 2026-07-04 · Owner: Detoro bfb737ff · authority: in-loop
Task: `skills-task-verbs` (this program is itself the first task object on the board)

## Why

ADR 0008 shipped five systems (task objects, gate ledger, watch/notify, lane
manager + commit guard, LaneBoard) and its Consequences section promised "the
Leadership/Collaboration skill protocol shrinks to CLI verbs" — but no lane
owned that prose change. Result observed 2026-07-04: `conclave task list` =
`[]`, zero skill sidecars mention `conclave task`/`conclave lane`, and the two
post-ADR lanes (term-remount, guard-hookspath-warn) ran on legacy bb keys.
The machinery shipped; adoption did not. This is a lead plan defect, owned.

## Decision being encoded (protocol ruling, final)

Work items live in the TASK SYSTEM; the blackboard remains for ad-hoc durable
facts only (conventions, anomalies, notes, constraints). Concretely:

| was (bb convention)              | is (CLI verb)                                          |
|----------------------------------|--------------------------------------------------------|
| `claim:<task>` key               | `conclave task claim <ws> <slug>` (or `lane start`)    |
| `plan:<task>` key (owner, authority) | `task create … --plan-file` — plan body carries `owner:` + `authority: in-loop`; read via `task get` |
| `progress:<task>` key            | `conclave task note <ws> <slug> <text>`                |
| self-reported "gates green"      | `conclave task gate <ws> <slug> -- <cmd…>` (system-recorded exit code + HEAD SHA + output tail) |
| challenge via `tell` prose       | `conclave task challenge … --claim --evidence --proposal --default [--deadline-min N]`; lead answers `task rule`; expired deadline fires the stated default automatically |
| polling progress keys            | `conclave task watch <ws> <slug>` (injected notifications; stall engine alerts owner) |
| hand-rolled worktree lifecycle   | `conclave lane start/finish <ws> <slug>`; commit scope enforced by `conclave lane guard install` + `$CONCLAVE_COMMIT_SCOPE` |

States: `planned → claimed → in_progress → review → merged` (+ `abandoned`),
moved with `task state`. `task close` prints the memory-save reminder.

## Scope (file boundary)

Exactly these, prose-only, surgical edits that keep each skill's voice:

- `src-tauri/skills/agent-loop/SKILL.md` — authority lives on the task;
  challenges/rulings via `task challenge`/`task rule`; progress via `task note`.
- `src-tauri/skills/collaboration/SKILL.md` — claiming via `task claim`/`task
  list`; bb hygiene section names what STAYS on the bb.
- `src-tauri/skills/implementer/SKILL.md` — claim/read via `task get`; gates
  via `task gate`; escalation via `task challenge`; report via `task note`;
  close via `task state` + `task close`; lanes via `lane start/finish`.
- `src-tauri/skills/leadership/SKILL.md` — `task create` (plan-file, boundary,
  canon) as the delegation record; oversight via `task watch` + stall engine;
  gate-ledger evidence at acceptance; rulings via `task rule`; solo work still
  claims a task; multi-lane boundaries via `--boundary` + commit guard.
- `src-tauri/skills/memory/SKILL.md` — "Memory is not the blackboard" bullet
  now says live coordination = task system + bb ad-hoc facts.
- `src-tauri/skills/strategic-compact/SKILL.md` — restore-verify step adds
  `task list`/`task get` next to `bb get`.
- `src-tauri/skills/arta-designer/SKILL.md` — design handoff pins the canon on
  the task (`--canon`) when one exists; bb `design:*` stays the pre-task home.

Out of scope: any Rust/TS code, the bundled `Resources/` copy (rides the next
rebuild per plan:rebuild-0.2.0), teaching the HUMAN's docs.

## Gates

- `cargo test` in `src-tauri` (frontmatter of every SKILL.md must still parse;
  skill tests use an override dir so content is free to change — the gate
  catches structural breakage only). Run via `conclave task gate` so the
  evidence is on the ledger.
- Mellow review of the lane diff BEFORE merge (blocking): checks every table
  row above against actual CLI verbs (`conclave help` output is the spec), no
  stale `claim:`/`plan:`/`progress:` work-item reference survives outside the
  explicit "what stays on the bb" lists, and prose voice matches each file.

## Risk ledger

- Frontmatter is load-bearing (`mandatory:` flag, ADR 0003) — do not touch it.
- The sidecars agents read TODAY regenerate from the INSTALLED app's bundled
  skills; this change reaches agents only after the next rebuild+relaunch.
  Interim: nothing — human chose the durable layer over the live push.
- `task claim`/`state`/`note`/`gate` are agent-context verbs; exact flag
  syntax must be copied from `conclave help`, not from memory.

## POST-LAND ledger (2026-07-04)

- MERGED @ 3274567 (lane head 5aab9ab). Mellow LAND PASS 0 blocking @ 8b3c8bf;
  his one nit (collaboration offered `task state … merged` to implementers,
  but `merged` via `state` is review-only per repo/task.rs:120 and the
  integrator's move by protocol) fixed in 5aab9ab — credit: Mellow.
- Gates on the task ledger (first task object ever on the board): cargo test
  exit 0 @ 8b3c8bf, @ 5aab9ab, and post-merge @ 3274567 (lead-reproduced).
- Deferred Low: implementer/SKILL.md says "`task close` ends it" right after
  describing the integrator's `state merged` — a reader who runs BOTH gets
  "cannot close a task in state 'merged'" (close is the claimed/in_progress/
  review → merged shortcut, task.rs:452). Harmless error, prose could pick
  one verb per path. Fold into the next skill-prose pass.
- Adoption note: prose reaches agents when sidecars regenerate from the
  INSTALLED app's bundled skills — i.e. next rebuild + relaunch. Until then
  agents follow the old bb protocol; the lead bridges by creating tasks.

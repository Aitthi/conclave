# Plan: Agent Work System v1 (ADR 0008)

Owner/lead/escalation: Detoro `bfb737ff` · authority: in-loop (human green-light 2026-07-04).
Reading order for every implementer: ADR 0008 → this plan (GLOBAL CONSTRAINTS + your lane) → cited source files.
Blackboard: claim `claim:aws/<lane>`, progress `progress:aws/<lane>`. Reviews land at `review:aws/<lane>`.

## Lane map

| Lane | Scope | Who | Depends on |
|---|---|---|---|
| A | task tables + repo + `task.*` commands + CLI verbs + gate runner + memory nudge | Dew `40d90aed` | — |
| B | watch/notify injection + stall timer + challenge-default timer | Dew (after A) or Marty `5524d1c5` after 21:26 | A merged |
| C | `conclave lane start/finish` + pre-commit guard | Guetta `2b110fd3` | — (parallel) |
| D | LaneBoard.tsx + telemetry strip + `task:changed` bus/ipc event | Tiësto `fd0dec79` | Arta canon; builds against §Frozen interfaces with mock data until A merges |
| D-canon | design proto for lane board + telemetry strip | Arta `688719b6` | — (parallel, first) |

Dabin/Marty: NO assignments before 21:26 (bb `constraint:dabin-usage-limit`).
Mellow `4b13a0e6`: LAND review at each lane's merge gate; lead reproduces gates before integrating.
Rebuild r5 (with all of this + memory-graph) is cut at program end; r4 install stays HELD.

## GLOBAL CONSTRAINTS (every lane inherits)

- Shared-tree git: NEVER plain `git commit` in the shared checkout — `git add <files>` then `git commit -m "..." -- <files>` (`-m` BEFORE `--`). Implementers work in `.claude/worktrees/<lane>` on branch `lane/aws-<lane>` cut from main (`git worktree add -b lane/aws-<lane> .claude/worktrees/aws-<lane> main`). Lead merges; nobody self-merges.
- Gates per lane, run in your worktree: `cargo test --lib` AND `cargo clippy --all-targets -- -D warnings` (— `--all-targets` is load-bearing, test-file lints escape the lib build). Frontend `npx tsc --noEmit` runs in the MAIN repo post-merge (bare worktrees lack node_modules; a worktree run gives false module-not-found).
- Data layer: sqlx(sqlite) + the repo chain-builder pattern — copy the shape of `engine/repo/blackboard.rs`; do NOT introduce rusqlite.
- UDS command handlers: copy the shape of `engine/commands/blackboard.rs`; register verbs in `engine/router.rs` dispatch match.
- CLI: extend `src-tauri/src/bin/conclave-cli.rs`; self-id comes from `CONCLAVE_INSTANCE_ID` (see existing `require_self`), workspace id is always an explicit arg (matches `bb`/`memory` style).
- All UI copy in English. camelCase wire fields; `engine/bus.rs` payloads must stay in sync with `src/ipc/events.ts` (serialisation tests enforce).
- Escalation: design/spec conflicts → Detoro, ruling final. Implementation judgment within plan intent → yours, log it in `progress:aws/<lane>`.

## Frozen interfaces (build against these; changing them requires a Detoro ruling)

### Migration `src-tauri/src/engine/migrations/0012_task_system.sql`

```sql
CREATE TABLE task (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  slug TEXT NOT NULL,
  title TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'planned'
    CHECK (state IN ('planned','claimed','in_progress','review','merged','abandoned')),
  owner_agent_id TEXT,
  implementer_agent_id TEXT,
  file_boundary TEXT NOT NULL DEFAULT '[]',
  design_canon TEXT,
  plan TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (workspace_id, slug)
);
CREATE TABLE task_event (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('note','state','gate','challenge','ruling')),
  actor_agent_id TEXT,
  payload TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);
CREATE INDEX idx_task_event_task ON task_event(task_id, created_at);
CREATE TABLE task_watch (
  task_id TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  PRIMARY KEY (task_id, agent_id)
);
```

### UDS commands (router names) and CLI verbs

| Router cmd | CLI | Notes |
|---|---|---|
| `task.create` | `conclave task create <ws> <slug> <title...> [--boundary p1,p2] [--canon txt] [--plan-file path]` | owner = caller if spawned, else arg `--owner` |
| `task.list` | `conclave task list <ws> [--state s]` | JSON array, event counts included |
| `task.get` | `conclave task get <ws> <slug>` | task + last 20 events |
| `task.claim` | `conclave task claim <ws> <slug>` | sets implementer=self, state→claimed |
| `task.setState` | `conclave task state <ws> <slug> <state>` | records `task_event(kind='state')` |
| `task.note` | `conclave task note <ws> <slug> <text...>` | replaces free-text progress keys |
| `task.gate` | `conclave task gate <ws> <slug> -- <cmd...>` | CLI runs cmd, captures exit code, `git rev-parse HEAD`, last 2000 bytes of output; payload `{"cmd","exit","sha","tail","cwd"}` |
| `task.challenge` | `conclave task challenge <ws> <slug> --claim t --evidence t --proposal t --default t [--deadline-min N]` | payload carries all five; deadline absent = advisory |
| `task.rule` | `conclave task rule <ws> <slug> <challengeEventId> <text...>` | kind='ruling', payload `{"challengeId","text","by":"<agentId>"}` |
| `task.close` | `conclave task close <ws> <slug>` | state→merged; CLI then prints memory nudge: "Boundary reached — what did this cost to learn that the repo doesn't record? `conclave memory remember <ws> ...`" |
| `task.watch` / `task.unwatch` | `conclave task watch\|unwatch <ws> <slug>` | inserts/removes `task_watch` row for self |

Wire shape of a task (camelCase): `{"id","workspaceId","slug","title","state","ownerAgentId","implementerAgentId","fileBoundary":[],"designCanon","plan","createdAt","updatedAt"}`. Lane D codes against this EXACT shape.

### Bus event (Lane D + B)

`engine/bus.rs`: `pub const TASK_CHANGED: &str = "task:changed";` payload `TaskChanged { workspace_id, task_id, slug, state }` (camelCase serde) + mirror in `src/ipc/events.ts` EVENT_NAMES/interface. Emitted by `task.*` mutating handlers (Lane A emits; B reuses).

## Lane A — task core (Dew)

Files: migration above; `engine/repo/task.rs` (+`pub mod task;` in `engine/repo/mod.rs`); `engine/commands/task.rs` (+mod +router entries); CLI parsing in `bin/conclave-cli.rs` (follow the `memory`/`bb` arm patterns, incl. `--` handling for `gate` — note from memory: message flag BEFORE `--` when you commit).
Gate-runner detail: run via `std::process::Command` `sh -lc <joined cmd>` in caller cwd, capture combined output, DON'T stream; truncate tail to 2000 bytes; non-zero exit still records (a red gate is evidence too).
Tests: repo CRUD + state-machine transition validation (reject e.g. merged→claimed) + gate payload serialisation; router smoke like blackboard's.
Acceptance: all CLI verbs round-trip against a live engine (`conclave task create/claim/gate/get`), gates green, Mellow LAND, lead reproduces.

## Lane B — watch/notify + timers (after A merges)

Files: `engine/commands/task.rs` (notify hook), new `engine/runtime/task_timer.rs` (+wire into engine startup where the embedder/timer tasks spawn — find the tokio::spawn cluster in `engine/mod.rs`/`state.rs`).
Behavior: on mutating `task.*`, for each `task_watch` row (except actor) inject one line via the `message.inject` path: `[task <slug>] <actor-name>: <kind> — <summary>`. Timer every 5 min: (1) stall = state claimed/in_progress AND newest task_event older than 30 min → inject alert to owner once per hour max (track last-alert in memory, not DB); (2) challenge deadline passed with no ruling whose payload.challengeId matches → insert default ruling event + notify actor and owner.
Tests: timer logic pure-fn tested (pass now-timestamp in); injection path mocked.
Acceptance: watch a task from a second agent, mutate from first, line arrives in watcher's PTY; deadline default fires in a test with injected clock.

## Lane C — lane manager + commit guard (Guetta)

Files: CLI-only — new module or inline arms in `bin/conclave-cli.rs`: `lane start <ws> <slug>`, `lane finish <ws> <slug>`, `lane guard install`.
`lane start`: `git worktree add -b lane/<slug> .claude/worktrees/<slug> main` (this exact form — plain `add` fails "main is already used by worktree"), then `task claim` + `task state in_progress` via UDS if the task exists (soft-fail with warning if not).
`lane finish`: refuse if worktree dirty; `git worktree remove .claude/worktrees/<slug>` + `git branch -d lane/<slug>` (only after merge — `-d` not `-D` is the safety).
Guard hook (`.git/hooks/pre-commit`, installed by `lane guard install`): skip when in a lane worktree (`git rev-parse --git-dir` ≠ `git rev-parse --git-common-dir` parent) — hooks are SHARED across worktrees via common dir, this check is load-bearing. In the shared checkout: if `$CONCLAVE_COMMIT_SCOPE` unset → reject with instructions; else every `git diff --cached --name-only` path must match one scope pathspec prefix, else reject naming the offender. Keep the hook POSIX sh.
Tests: shell-level fixture repo test script under `src-tauri/tests/` or a `#[test]` driving a temp git repo via std::process.
Acceptance: guard demonstrably blocks the b9ab709 replay (stage foreign file, commit with scope → rejected) and allows scoped commit; worktree lifecycle round-trips.

## Lane D — LaneBoard UI + telemetry strip (Tiësto, after Arta canon)

DESIGN CANON: Arta's proto under `.arta/proto/screens/` at the SHA Arta pins on bb `design:aws-laneboard` — fidelity target, escalation for design questions → Arta.
Files: `src/components/LaneBoard.tsx`; ipc additions `src/ipc/` (task.list/get + `task:changed` in `events.ts`); mount point per canon (likely alongside Blackboard/MemoryGraph views in AppShell routing — match how MemoryGraph.tsx was mounted).
Telemetry strip: aggregate existing `session:context` events per workspace (ContextBars.tsx already consumes them per-session — reuse its data path, don't invent a new one).
Until A merges: develop against a local mock returning the frozen wire shape; swap to real `task.list` at integration.
Acceptance: tsc 0 in main repo post-merge, canon fidelity gate by Arta (token parity vs sibling components counts, per memory), Mellow LAND.

## Risk ledger

- Worktree hooks share `.git/hooks` via common dir → guard MUST self-skip in lanes or it bricks every lane commit (Lane C acceptance covers).
- `message.inject` into a WORKING agent interrupts it — reuse whatever queueing `tell` already does (`engine/commands/message.rs`); do not invent a second injection path.
- `task.gate` runs arbitrary shell as the calling agent: it's the agent's own privilege, no escalation — but NEVER run it engine-side.
- Editor ghost tsc errors after worktree teardown are stale LSP noise, not real (verify with `git merge-base --is-ancestor`).
- sqlite has no ALTER-friendly CHECK evolution: if states change later, that's a new migration, so challenge the state list NOW if you disagree.
- CLI `--` parsing: `gate`'s `-- <cmd...>` must not eat task flags; parse flags before the `--` split.

## Post-land

r5 rebuild (Dew) off main including this program + memory-graph; then install (supersedes held r4). Memory: each lane saves its hard-won lessons at close (`conclave memory remember`).

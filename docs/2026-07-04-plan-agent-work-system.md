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

Response shapes (RULED 2026-07-04, challenge credit: Tiësto — these are final):
- `task.list` res = `Task[]`, each row = frozen task shape PLUS `"eventCount": number` PLUS the two derived board fields below (RULED 2026-07-04 #2, escalation credit: Tiësto — canon lane-board.tsx renders gate badges + challenge chips on every card; N+1 `task.get` per card rejected as wasteful and racy):
  - `"lastGates": { "cmd", "exit", "sha", "createdAt" }[]` — the newest `kind='gate'` event PER DISTINCT `cmd`, most-recent-first, capped at 6 distinct cmds; always present, `[]` when none. (AMENDED 2026-07-04 from single `lastGate?` — Arta fidelity finding F1: the canon renders a badge per gate kind so a card can show "test green + clippy red" simultaneously; newest-per-cmd is that exact semantics and stays bounded. Full history stays in `task.get`.) Badge label derives client-side from `cmd`.
  - `"challenges": { "id", "status": "open"|"ruled", "claim", "deadlineAt"? }[]` — derived from `kind='challenge'` events joined to `kind='ruling'` by `payload.challengeId`; always present, `[]` when none. `deadlineAt` is an ISO timestamp (client computes minutes remaining live), never a server-computed countdown.
- `task.get` res = `{ "task": Task, "events": TaskEvent[] }` — last 20 events, sorted `createdAt` DESC (newest first).
- `TaskEvent` = `{ "id", "taskId", "kind", "actorAgentId", "payload", "createdAt" }`, `kind` ∈ note|state|gate|challenge|ruling; `payload` is a JSON OBJECT on the wire (engine parses the stored TEXT column; unparseable → `{}`), never a double-encoded string.
- `task:changed` event = `{ "workspaceId", "taskId", "slug", "state" }` (as §Bus below).
- Optional wire fields (`ownerAgentId`, `implementerAgentId`, `designCanon`, `actorAgentId`, `deadlineAt`) are OMITTED when absent, never `null` (arrays — `lastGates`, `challenges` — are instead ALWAYS present, `[]` when empty) (RULED 2026-07-04, ratifying Dew judgment #3; Mellow evidence: faithful mirror of the BlackboardEntry precedent — server omits `blackboard.rs:42`, TS `lastWriterId?:` `types.ts:175`). TS consumers type them `?:` and use loose `== null` checks.
- Challenge deadline STORAGE (RULED 2026-07-04, surfaced by Tiësto's pre-merge audit of 081b0d5): `--deadline-min N` stays as CLI input sugar, but the engine converts it to an absolute `deadlineAt` ISO timestamp AT INSERT and stores that in the challenge payload. Everything downstream (Lane B deadline timer, list derive, task.get) reads `deadlineAt`; nothing re-derives from relative minutes.
- `conclave task gate` PROPAGATES the gate command's exit code as its own process exit code, after recording (RULED 2026-07-04, credit: Mellow latent-Low). Recording red and exiting 0 hides failure from scripting agents — `conclave task gate <ws> <slug> -- cmd && <next>` must short-circuit on red.

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

### POST-LAND ledger

- **PROGRAM CODE-COMPLETE @ 7ab08a7** — all four lanes landed and PASSed (A 29a1053, B 7ab08a7, C ef07b35, D 657d7b4). Remaining: r5 rebuild + install (Dew), live-pixel confirms for LaneBoard + memory-graph ride the relaunch.
- **Lane B MERGED @ 7ab08a7** (4dcc48d, Dew; Mellow LAND PASS 0 open after one CHANGES-REQUESTED round: AUTO markers on the 2 timer texts — blocking per lead's attribution-ruling condition — and TASK_CHANGED emit after auto-default ruling, both fixed + regression-tested; lead-reproduced 563 lib / 40 bin / clippy clean). Design note on record: timer notifications are attributed to a real involved party (`from_instance_id` NOT NULL, no system identity) and carry an explicit `AUTO` body marker; `notify_watchers` is attributed to the real acting verb-runner, no marker by design.

- **Lane A MERGED @ 29a1053** (ebddbf8, Dew; Mellow LAND PASS held-lifted 0 blocking; lead-reproduced 547 lib / 40 bin / clippy clean; post-merge tsc 0 + vite clean). Three mid-flight spec amendments (derive-in-list, gate rc propagation, deadlineAt-at-insert + lastGates per-cmd) absorbed with zero drift findings. Latent Low from first LAND round, superseded: gate rc propagation was ruled IN before merge, no open items.
- **Lane D MERGED @ 657d7b4** (5052a84, Tiësto; Arta fidelity ACCEPT @ canon fa4929b, F1 resolved via lastGates amendment; Mellow LAND PASS 0 blocking; lead-ran tsc 0 + vite clean on merged main). Cross-lane A↔D wire contract verified closed both sides. Caveat: live-pixel confirm defers to r5 relaunch (running app is an old inode — memory-graph precedent). Doc-rot nit (mock header `lastGate?`) fixed at integration.
- **Lane C MERGED @ ef07b35** (e1ab408, Guetta; Mellow LAND PASS 0 blocking; lead-reproduced 475/21/4 + clippy clean). Two latent Lows on record, deferred follow-ups, non-blocking:
  - L1: `lane guard install` writes `<common>/hooks/pre-commit` directly and does not honor `core.hooksPath` — silent no-op if a checkout ever sets it (e.g. husky). Inert in this repo today. Follow-up: warn-if-set at install time.
  - L2: hook `while read` mishandles git-quoted paths (embedded newline in filename) — theoretical bypass on pathological filenames only, non-adversarial input, negligible.
  - Hard-won (saved to memory by Guetta): worktree self-skip must compare RAW `git rev-parse --git-dir` vs `--git-common-dir`, never absolutised paths — macOS `/tmp`→`/var` symlinks desync the compare and falsely skip the shared checkout.

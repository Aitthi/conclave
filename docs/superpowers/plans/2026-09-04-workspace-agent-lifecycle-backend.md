# Workspace and agent lifecycle backend

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Add persistent, race-safe Start/Stop lifecycle controls for a workspace and Stop/Resume controls for individual workspace agents. Stopping retains identity, configuration and durable history; it only prevents new work and tears down live runtimes.

## Product contract

- `workspace.run_state` is `started | stopped`; `workspace_agent.availability` is `active | stopped`.
- Fresh and upgraded user workspaces default to `stopped`, matching the human requirement that first entry is inspect-only and never launches agents before an explicit Start. Hidden skill-draft/internal workspaces that require a runtime must be created/transitioned explicitly as `started`.
- A stopped workspace can still be selected and inspected. Metadata management is not frozen; “inspect-only” means no agent-runtime-producing actions, terminal/chat input, agent messages or task claims.
- `Start workspace` persists `started`, then launches only individually `active` agents in stable `(added_at,id)` order. It skips individually stopped agents. It is idempotent and returns deterministic successes/skips/failures; one launch failure does not roll back successful siblings or the workspace state.
- `Stop workspace` persists `stopped` as the linearization point, clears restart arms, sets transient agent statuses idle, tears down all live runtimes, and marks browser tabs ended. It never changes individual availability. It is idempotent.
- `Stop agent` persists `availability=stopped`, clears its restart arm, tears down its live runtime, sets transient status idle and retains its workspace-agent row, session id, role/skills/supervisor/settings, snapshots, tasks, messages and history.
- `Resume agent` is allowed only in a started workspace. It launches a fresh runtime generation using the existing 1:1 session and changes availability to active only when launch succeeds; failure leaves it stopped. It is idempotent.
- Generic spawn/restart, message send/inject and task claim require `workspace=started && agent=active`. Rejection happens before queuing/delivery/mutation. Existing ownership, watches, historical and queued rows remain.
- Direct CLI/chat process context is fresh after Resume; do not claim terminal/provider chat transcripts are persisted.
- Remove/delete stay distinct destructive operations and continue cascading as today.

## Persistence and typed contracts

1. Add migration `src-tauri/src/engine/migrations/0027_workspace_lifecycle.sql` with additive constrained columns and no table rebuild:
   - `workspace.run_state TEXT NOT NULL DEFAULT 'stopped' CHECK (run_state IN ('started','stopped'))`
   - `workspace_agent.availability TEXT NOT NULL DEFAULT 'active' CHECK (availability IN ('active','stopped'))`
2. Register v27 and extend fresh/contiguous/v26-upgrade tests. The upgrade fixture must prove all existing relations survive and existing user workspaces become stopped.
3. Thread fields through every repository column list/query/insert/return model. Add narrow repository setters and eligibility lookup(s) rather than scattering raw SQL.
4. Add wire commands:
   - `workspace.start { workspaceId } -> { workspace, readyAgentIds, skippedStoppedAgentIds, failures }`
   - `workspace.stop { workspaceId } -> { workspace, stoppedRuntimeIds }`
   - `instance.stop { workspaceAgentId } -> void`
   - `instance.resume { workspaceAgentId } -> Session`
5. Add `workspace:changed { workspaceId, runState }`; retain existing per-session and roster events.
6. Add CLI syntax/help/parser/dispatch: `ws start <workspaceId>`, `ws stop <workspaceId>`, `agent stop <workspaceAgentId>`, `agent resume <workspaceAgentId>`.

## Concurrency and implementation structure

- Add keyed async workspace lifecycle locks to `AppState`. Workspace start/stop/delete takes WRITE; per-agent spawn/stop/resume/restart/remove and target message delivery take READ, followed by a keyed per-agent lock where needed. Lock order is always workspace then agent; never hold `std::sync::Mutex` across await.
- Batch internals execute under an already-held workspace WRITE lock and must not reacquire it. Extract a launch helper and a teardown helper that accept the held lifecycle context.
- Workspace Stop uses teardown without changing individual availability. Agent Stop wraps teardown and changes availability.
- Persist workspace Stop before unregistering runtimes. Start persists started before launches. Resume changes availability only after successful registration (or rolls it back before returning an error).
- Keep runtime epochs as the late-EOF guard. Every launch and detached restart tail must re-read workspace run state and agent availability immediately before registration.
- Clear pending restart arms during both workspace and agent Stop so a snapshot-save tail cannot revive stopped state.
- Fold workspace delete, instance remove, skill-draft cleanup and agent-definition delete through the lifecycle helpers. If multiple workspace WRITE locks are needed, acquire/process workspace ids in sorted order.
- Define message-vs-Stop by holding the workspace READ guard through eligibility check and delivery/persist body: an operation that wins before Stop completes; operations after Stop reject and create no queued row.
- Add a browser registry `mark_resumed`/equivalent so a successfully resumed generation is not permanently shown as ended.

## Exact boundary

- `src-tauri/src/engine/migrations/0027_workspace_lifecycle.sql`
- `src-tauri/src/engine/db.rs`
- `src-tauri/src/engine/repo/workspace.rs`
- `src-tauri/src/engine/repo/workspace_agent.rs`
- `src-tauri/src/engine/state.rs`
- `src-tauri/src/engine/runtime/browser_tabs.rs`
- `src-tauri/src/engine/commands/workspace.rs`
- `src-tauri/src/engine/commands/instance.rs`
- `src-tauri/src/engine/commands/message.rs`
- `src-tauri/src/engine/commands/task.rs`
- `src-tauri/src/engine/commands/agent.rs`
- `src-tauri/src/engine/commands/skill_draft.rs`
- `src-tauri/src/engine/commands/cli.rs`
- `src-tauri/src/engine/router.rs`
- `src-tauri/src/engine/bus.rs`
- `src-tauri/src/bin/conclave-cli.rs`

If compilation proves one additional backend module is structurally required, file a task challenge before editing outside this boundary. No `src/` UI files belong to this lane.

## Required tests

- Migration: v26 populated workspace/agents/session/supervisor/tasks/messages/history upgrades intact; existing workspace is stopped; agent active; invalid values rejected; fresh DB v27; hidden draft workflow works.
- Commands: workspace stop live/dead/double; workspace start stopped/double/partial failure; active-only launches; agent stop live/dead/double; agent resume success/double/failure; same ids/history; generic spawn/restart guards; delete/remove regressions.
- Work guards: stopped workspace or agent rejects message send/inject without queued insert and rejects task claim without changing owners/watches.
- Races: workspace Stop vs agent Resume/spawn/message/restart tail; Start vs Stop; late EOF; fixed multi-workspace delete lock order.
- CLI/router/bus: exact arity, help, dispatch, camelCase serialization and event payload.

## Verification and handoff

- Run `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`.
- Run targeted lifecycle/migration/CLI tests while iterating.
- Run `cargo test --manifest-path src-tauri/Cargo.toml` before READY.
- Commit only the boundary using `conclave stage commit ...`; attach commit SHA and exact green commands in the READY note.
- Implementation judgment within this contract belongs to the implementer. Any contract/boundary conflict is a task challenge to Aoki, final.

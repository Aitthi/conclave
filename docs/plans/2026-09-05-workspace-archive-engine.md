# Reversible workspace archive engine and IPC
owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop
Implementer: Dabin (21a29434-cfd8-4b6e-a519-4f2a2a29d093). Aoki rules and integrates. Authority: user requested Archive workspace, retain existing permanent Delete.

## Reading order
PRODUCT.md → docs/research/2026-09-05-workspace-overview-archive.md section 2 → this plan → existing workspace/instance lifecycle tests. This plan rules over research proposals when they differ.

## Settled behavior
Archive is reversible organisation, distinct from hidden scratch workspaces and run_state. Add nullable archived_at (UTC RFC3339), default NULL; preserve existing rows and migrations. Reserve migration 0029_workspace_archive.sql; later usage engine uses 0030+. Do NOT reuse hidden or delete child records/files.
Eligibility: archive only non-hidden, run_state=stopped, with no live runtimes. Started even with zero current runtimes must be stopped explicitly first. Return an actionable Invalid error (Stop workspace before archiving). No implicit stop, no agent termination. Existing archived target is idempotent success (return current row, no repeated event); missing is NotFound; hidden is Invalid.
Hold workspace lifecycle write guard around fresh validation plus transaction. A concurrent active draft/fusion must not slip through: these one-shot execution routes take the matching workspace read guard for their operation and recheck archive status after obtaining it. Archive may use try_write with actionable busy rejection to avoid hanging behind a long one-shot; it must never terminate it. Follow workspace→agent lock order, no reentrant write/read acquisition. Checks must be in production AppState command paths, not only test helpers.
Transaction archives row, keeps stopped, normalizes transient agent status to idle, preserves availability and all sessions/notes/tasks/messages/artifacts/memory/project files. Restore clears archived_at and keeps stopped; no launch, no availability changes; idempotent already-active returns current row, hidden Invalid. Delete remains allowed for archived and uses existing teardown/cascade path.
Normal workspace.list excludes hidden+archived. New workspace.listArchived returns nonhidden archived rows ordered archivedAt descending, id tie-breaker. Internal get and historical joins still include archives. workspace.use and update reject archived (restore first). Do not promise immutable task/history ledgers: normal historical reads and late completion persistence remain possible; UI interaction returns through Restore.
Guard all ways to execute new model work or manipulate active membership on archived workspace: workspace.start; central runtime_eligibility/require_launch_eligible including resume/restart/compact detached continuation; input send/inject (reject before queuing, no new queued delivery); draft.run; fusion.run; addToWorkspace; public instance removal/position/availability actions as appropriate (internal permanent-delete cleanup must still work). Share a small archived eligibility helper; avoid scattering subtly different predicates. Do not change behavior for active stopped workspace draft/fusion beyond archive guard.

## Exact IPC contract
Workspace gains archivedAt?: string | null in TS (optional only for backward fixtures; engine serializes explicit null/string). Existing list/link/update/start/stop return same envelopes with this additive field.
workspace.listArchived: req void, res Workspace[]; ipc.workspace.listArchived().
workspace.archive: req {workspaceId:string}, res Workspace; ipc.workspace.archive(req).
workspace.restore: req {workspaceId:string}, res Workspace; ipc.workspace.restore(req).
workspace:changed retains workspaceId/runState and gains archivedAt?:string|null; engine emits explicit current archivedAt on lifecycle/archive/restore events. Frontend will refetch on archive/restore so never rely only on changing runState. Existing consumers remain source compatible.
Expose matching CLI ws archive <id>, ws restore <id>, ws list --archived (preserve existing ws list output semantics and usage). Route to shared engine handlers, do not duplicate decisions in CLI.

## File boundary
src-tauri/src/engine/migrations/0029_workspace_archive.sql
src-tauri/src/engine/db.rs
src-tauri/src/engine/repo/workspace.rs
src-tauri/src/engine/repo/workspace_agent.rs
src-tauri/src/engine/commands/workspace.rs
src-tauri/src/engine/commands/instance.rs
src-tauri/src/engine/commands/message.rs
src-tauri/src/engine/commands/draft.rs
src-tauri/src/engine/commands/fusion.rs
src-tauri/src/engine/commands/agent.rs
src-tauri/src/engine/commands/position.rs
src-tauri/src/engine/commands/cli.rs
src-tauri/src/engine/router.rs
src-tauri/src/engine/bus.rs
src-tauri/src/bin/conclave-cli.rs
src/ipc/types.ts
src/ipc/commands.ts
src/ipc/events.ts
No src UI, fixtures, docs/skills templates, lockfiles/dependencies or other files. If another exact path is necessary file challenge with evidence; Aoki amends boundary before write. Usage engine MUST wait for ARCHIVE ENGINE MERGED before touching shared files/migrations/IPC. Use isolated lane worktree, no self merge.

## Acceptance and evidence
Test migration existing data NULL + preservation of populated graph; active/archived/hidden lists and internal get; archive/restore idempotence; reject started/live/busy/hidden/not-found with no termination; preserve child rows/availability/files; restore stopped zero runtimes; spawn/resume/restart/input/add/draft/fusion bypass rejection; queued delivery cannot relaunch; archive-vs-spawn and archive-vs-one-shot serialization; permanent archived Delete remains possible. Reuse mock runtime/provider/oneshot fixtures; no real API calls or mutations of the running Conclave workspace.
Run cargo test --manifest-path src-tauri/Cargo.toml (report unrelated preexisting failures with evidence), pnpm build, and rustfmt on touched Rust files only (repo-wide fmt has known unrelated backlog). Tests must exercise production command guards, not only SQL helpers. Record task gates, commit via conclave stage commit. READY note exact ARCHIVE ENGINE READY with SHA, contract, gates, tested bypass list and remaining limitations. Aoki reviews/retests/merges and then posts ARCHIVE ENGINE MERGED to release usage engine.

# Plan: graph-live-refresh — Memory graph updates live via memory:changed bus event

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Task: `graph-live-refresh` · Implementer: Dew 40d90aed · Reviewer: Mellow (LAND, blocking)
Status: APPROVED by human 2026-07-05 ("ทำทั้งสองอันเลย" — this is fix 1 of 2;
fix 2 is `review-reminder`).

## Why

`MemoryGraph.tsx:167` fetches `memory.graph` once per `[workspaceId]` mount
and never again — chunks saved while the view is open never appear, which
read to the human as "no agent saves memory". The plumbing for live updates
already exists on both sides: the engine bus (`bus.rs`, `TASK_CHANGED`
precedent) and the frontend Tauri-listen helper (`src/ipc/events.ts`).
Connect them.

## Decisions (settled, encode exactly these)

1. New bus event `MEMORY_CHANGED` = `"memory:changed"` in
   `src-tauri/src/engine/bus.rs`, payload `{ workspaceId }`, emit helper
   mirroring the `TASK_CHANGED` pattern (bus.rs:194).
2. Emit AFTER every successful chunk-writing command in
   `src-tauri/src/engine/commands/memory.rs`: `remember`, `delete`, `clear`,
   and `approve` (the distiller-queue verb that writes a chunk). NOT
   `propose`/`reject`/`queue`/`search`/`status`/`graph` — they never touch
   `memory_chunk`, and the graph renders chunks only. A deduped remember
   (no row written) SHOULD still not emit.
3. Frontend: `src/ipc/events.ts` gains a typed `memory:changed`
   subscription following the existing helper pattern;
   `src/components/MemoryGraph.tsx` extracts its load logic into a
   refetchable callback and subscribes — refetch only when the event's
   `workspaceId` matches the prop. The existing `seq` guard (line 166-177)
   already makes overlapping loads safe; keep it.
4. On refetch, PRESERVE the simulation positions of nodes that still exist
   (carry x/y over by node id) so the graph doesn't re-explode on every
   save; new nodes spawn as they do today. If carrying positions turns out
   to fight the physics loop, a full reset is the acceptable fallback —
   note which you shipped.

## Tests / verification

1. Rust: bus emit unit coverage per the TASK_CHANGED tests' pattern (emit
   called on remember/delete/clear/approve success; NOT on deduped
   remember; NOT on propose/reject). If task.rs-style emit tests don't
   exist to mirror, assert via the commands' test harness with a probe.
2. Frontend: `npm run build` (tsc && vite build) green — the typed event
   addition compiles end-to-end.
3. Manual verify note on the ledger: with the dev app open on the graph,
   `conclave memory remember` from a terminal makes the node appear
   without reopening (this is the acceptance criterion; record what you
   observed — dev-mode `npm run dev` + `cargo tauri dev` is fine).

## Boundary

`src-tauri/src/engine/bus.rs`, `src-tauri/src/engine/commands/memory.rs`,
`src/ipc/events.ts`, `src/components/MemoryGraph.tsx`. Nothing else.

## Gates (commit first, then gate; from src-tauri unless noted)

- `cargo test` (full) · `cargo clippy --all-targets -- -D warnings`.
- From repo root: `npm run build` (tsc + vite).
- Mellow LAND (blocking): emit placement (after success, all four verbs,
  none on dedup/no-op), event payload/name consistency across the four
  files, subscription cleanup on unmount (no listener leak), seq-guard
  survives the refactor.

## Risk ledger

- Engine emit reaches live agents' app after rebuild; the frontend ships in
  the same bundle — one rebuild covers both.
- UDS-origin writes (CLI remember) must emit like UI-origin ones — task.rs
  events already do this via AppState; follow the same emit path, do NOT
  gate the emit on request origin.
- memory.rs is freshly merged distiller-queue code — rebase-level care;
  the emit is additive, no behavior change to the commands themselves.
- Do not touch task.rs/task_timer.rs (Tiësto's watch-filter lane owns them).

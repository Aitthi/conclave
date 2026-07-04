# ADR 0007 — Memory knowledge-graph view: data binding & edge derivation

Date: 2026-07-04 · Status: accepted · Owner: Detoro (lead) · Design: Arta, proto pinned @ 73ac6fa (superseded c134d01)

## Context

The human approved Arta's Obsidian-style graph view over the workspace semantic
memory store (`.arta/proto/screens/memory-graph.tsx`, snapshot
`.arta/snapshots/memory-graph.png`). The proto renders mock data; the store
(`src-tauri/src/engine/repo/memory.rs`) persists chunks with
`id / source_kind (manual|agent) / source_id / text / embedding / timestamps`
— **no links, no author name, no title**. Three open questions were deferred to
the lead in bb key `design:memory`.

## Decisions

1. **Graph binds the semantic store only.** Snapshots/handoffs stay in the
   ContextBars "Memory · snapshots" section. Rationale: snapshots are
   per-session coordination churn, memory is durable knowledge
   (memory-is-not-the-blackboard); folding them in would mix lifecycles and
   authors ambiguously.
2. **Edges are derived at query time, backend-side, never stored.**
   - `wiki` (solid): two chunks share at least one identical `[[token]]`
     (case-insensitive, trimmed) parsed from `text`. Today's real chunks have
     zero tokens → zero solid edges; that is honest and forward-compatible —
     the day agents write `[[links]]` into memories, edges appear.
   - `related` (dashed): cosine similarity between stored embeddings, top-k
     per node above a threshold (tuned against the real store; see plan).
   Rejected: persisting an edge table (premature schema for fully derivable
   data; invalidation cost on every upsert/delete).
3. **Author = spawned-agent identity, stamped at save time.** `conclave-cli`
   already knows `CONCLAVE_INSTANCE_ID` (used by `tell`/`snapshot`);
   `memory remember` will pass it so chunks save as
   `sourceKind:"agent" + sourceId:<instanceId>`. The backend request shape
   already accepts this — only the CLI/`map_argv` path is extended. Existing
   chunks (all `manual`, null sourceId) render in the neutral "Shared" group;
   we do NOT backfill or fabricate authors.
4. **New workspace-level destination.** The view mounts as a center-pane
   screen in `AppShell` following the Blackboard toggle pattern; it does not
   replace the per-agent snapshots section.
5. **No graph library.** react-flow (present) renders boxed nodes-with-handles
   — wrong look — and has no resolvable type decls; d3-force is not installed
   and stays out. The proto's hand-rolled ~40-line SVG force sim is the
   implementation. Rejected: adding d3-force (a dep for 40 lines we already
   have, curated-deps rule).

## Deferred — RESOLVED by Arta @ 73ac6fa (2026-07-04)

- "Shared protocol" hue → macOS system purple `#bf5af0` (new `--color-a-violet`
  token), clearly off the sky/indigo cluster.
- Rail button → Memory destination with the Network glyph, directly below
  Blackboard (Blackboard → Memory → Chat), pairing the two record surfaces.

## Consequences

- IPC gains `memory.graph` (contract frozen in
  `docs/2026-07-04-plan-memory-graph-view.md`); the CLI allowlist gains the
  instance-stamped `memory remember` mapping.
- Graph cost is O(n²) similarity at query time — fine at the store's scale
  (hundreds); revisit only if a workspace exceeds ~2k chunks.

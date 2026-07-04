# Plan — Memory knowledge-graph view (2 lanes)

Owner: Detoro (lead, bfb737ff-486d-4581-b407-95711d5e07ab) · authority: in-loop
Decisions: `docs/adr/0007-memory-graph-view.md` · Design record: bb `design:memory`

## DESIGN CANON

`.arta/proto/screens/memory-graph.tsx` + `.arta/proto/lib/memoryGraph.ts` +
`.arta/proto/components/AppShell.tsx` (rail placement) +
`.arta/snapshots/memory-graph.png`, pinned @ **73ac6fa** (supersedes c134d01:
Shared-protocol violet `--color-a-violet` #bf5af0 + Memory rail button, Network
glyph, directly below Blackboard). Design escalations →
Arta (688719b6-741d-43e1-bc6c-9a2e78d4e21b). Spec/plan escalations → lead.
Implementation judgment within this plan's intent: implementer's, logged in the
lane's progress key, never escalated.

## GLOBAL CONSTRAINTS (every task inherits)

- Build against the canon @ 73ac6fa. No visual improvisation; token parity
  with shipped siblings over literal class copying (see role-picker lesson in
  workspace memory).
- **No new dependencies.** Do not import react-flow (present but wrong look,
  no resolvable types). Do not add d3-force. The proto's hand-rolled SVG force
  sim is the implementation.
- UI copy is English.
- Shared-tree git discipline: work in your own worktree —
  `git worktree add -b lane/<task> .claude/worktrees/<task> main`.
  Commit with message BEFORE pathspec: `git commit -m "…" -- <files>`.
  Never plain `git commit`.
- Claim your lane (`claim:<lane key>` = your id) before touching files; update
  `progress:<lane key>` at each task boundary.
- Lead merges; no implementer merges to main.

## FROZEN IPC CONTRACT (changing this requires a lead ruling)

```ts
"memory.graph": {
  req: { workspaceId: string };
  res: {
    nodes: Array<{
      id: string;                       // chunk UUID — NOT a slug
      text: string;
      sourceKind: "manual" | "agent";
      sourceId: string | null;          // instance id when sourceKind=agent
      createdAt: string;
      updatedAt: string;
    }>;
    edges: Array<{ a: string; b: string; rel: "wiki" | "related"; score?: number }>;
  };
}
```

Empty store → `{ nodes: [], edges: [] }` (NOT an error).

---

## LANE A — backend (key: `memory-graph/backend`)

Files owned: `src-tauri/**` only. Out of bounds: `src/`, `.arta/`.

- **A1** `repo/memory.rs`: a chunk+embedding listing for one workspace
  (reuse/extend `list_embeddings` paging or add a capped list — implementer
  judgment; must also return `updated_at`, which `MemoryEmbeddingRow` lacks).
- **A2** `commands/memory.rs`: `graph` handler.
  - Decode embeddings via `runtime/vec_codec::decode` with the index row's
    dimension; propagate decode/dimension errors as `AppError`.
  - `wiki` edges: parse `\[\[([^\]]+)\]\]` from `text`, normalize
    (trim, lowercase); edge between chunks sharing ≥1 token.
  - `related` edges: cosine similarity over all pairs; per-node top-3 with
    score ≥ threshold; symmetric dedupe; skip pairs already `wiki`.
    Start threshold at 0.45 and TUNE against the real store (10 chunks in
    workspace 11ecf99b…): the result must be neither fully connected nor
    edgeless. Record the chosen value + resulting edge count in progress.
- **A3** `router.rs`: register `"memory.graph" => memory::graph`.
- **A4** Author stamping: `bin/conclave-cli.rs` + `commands/cli.rs map_argv` —
  when `CONCLAVE_INSTANCE_ID` is set, `memory remember` saves with
  `sourceKind:"agent"`, `sourceId:<instanceId>`; unset stays `manual`.
  Mechanism (argv flag shape) is implementer judgment; the `map_argv`
  allowlist stays explicit-arms-only, tests mirror the existing
  `tell_injects_sender_from_env` pattern.
- **A5** Gate in your worktree: `cargo test --lib` AND
  `cargo clippy --all-targets -- -D warnings`. Report exact counts as
  evidence.

## LANE B — frontend (key: `memory-graph/frontend`)

Files owned: `src/**` only. Out of bounds: `src-tauri/`, `.arta/` (read-only
canon). Buildable against the frozen contract before Lane A lands (mock the
call locally if needed; do not commit the mock).

- **B1** `src/ipc/commands.ts`: add the `memory.graph` entry (verbatim frozen
  contract) + `api.memory.graph` wrapper following siblings.
- **B2** `src/components/MemoryGraph.tsx`: lift the proto screen — force sim
  (module-scope `tick()`, pre-warm in `useLayoutEffect`, then Fit), hover
  neighbour-highlight, right detail card, floating panel (Search / Groups
  legend+counts / Forces sliders wired live), zoom(scroll)/pan(drag)/drag-node
  /Fit. Bind real data:
  - node label = first sentence of `text`, truncated ≤64 chars (derivation is
    client-side; the store has no titles).
  - radius by degree (from edges), colour by author: resolve `sourceId` →
    workspace-agent name+color via the roster IPC the `Roster` component
    already uses. `sourceId` null/unresolved → "Shared" group in violet
    `#bf5af0` per canon @ 73ac6fa (`--color-a-violet` in proto theme.css).
  - detail card links list = this node's edges → click walks to that node.
    Key EVERYTHING by chunk UUID — the proto's kebab slugs are mock-only
    (id-mismatch trap from workspace memory).
- **B3** Port the proto's `.gr-range` slider CSS into the app's real
  stylesheet under `src/styles/` — never import from `.arta/`.
- **B4** `AppShell.tsx` + `Rail.tsx`: Memory destination as a center-pane
  toggle following the Blackboard pattern exactly (state in AppShell, button
  in Rail, workspace-scoped, mutually exclusive with Blackboard/ChatHub).
  Placement per canon @ 73ac6fa: Network glyph, directly below Blackboard
  (Blackboard → Memory → Chat) — see `.arta/proto/components/AppShell.tsx`.
- **B5** States: 0 nodes → honest empty state (store empty / no workspace);
  search filter dims non-matches per proto.
- **B6** Gate: `npx tsc --noEmit` + `npm run build`. Worktrees have no
  `node_modules` — run `npm ci` in your worktree first; lead reruns both in
  the main repo post-merge regardless.

## INTEGRATION (lead)

Merge lanes → rerun all four gates in main repo → screenshot the live view vs
`.arta/snapshots/memory-graph.png` → Arta design gate → Mellow LAND review →
report. Relaunch caveat applies (installed app shows the view only after a
real relaunch — verify per workspace memory rule).

## RISK LEDGER

- Real store today: 10 chunks, ALL `manual`/null sourceId, zero `[[tokens]]`
  → one colour bucket and zero solid edges on first render. EXPECTED — do not
  fabricate authors or edges to make the demo prettier.
- all-minilm chunk-vs-chunk cosine may over/under-connect — that's why A2
  requires tuning against real data, not the constant as-committed.
- `MemoryEmbeddingRow` lacks `updated_at` — A1 must extend or add a row type;
  don't silently reuse `created_at` for both fields.
- Proto styles with `var(--color-*)`; the real app uses semantic Tailwind
  tokens — match shipped siblings, not proto class strings.
- After worktree teardown the LSP emits ghost tsc errors on the removed path —
  stale, not real.
- O(n²) similarity is fine at this scale; if a workspace ever exceeds ~2k
  chunks, cap and `log` the truncation — never truncate silently.

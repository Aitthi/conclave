# Plan: Built-in Design View (Arta core) + Artifacts View

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro, lead) · authority: in-loop
date: 2026-07-05 · status: approved by human (3 forks settled via interview)

## Mission (human's words, translated)

Bring the core design of Arta (`/Users/detoro/code/arta`) INTO Conclave, built-in
and vendor-neutral (not tied to any CLI vendor). A **Design view** shows the
design canvas full-page while the agent's CLI terminal stays visible on the
right. Also an **Artifacts view** for significant, self-contained agent outputs
(>15 lines, worth iterating/reusing: documents, code, HTML, SVG, diagrams,
React components). Do NOT add entries to the Roster footer menu (Add agent /
Blackboard / Memory / Chat / Lane board) — it is already crowded.

## Settled decisions (human-ruled 2026-07-05, do not re-open)

1. **Prototype engine = Vite sidecar.** The Arta viewer (a Vite dev server) is
   vendored into this repo and spawned/managed by the Rust engine; the Design
   view embeds it via iframe to `http://127.0.0.1:<port>`. REJECTED:
   HTML-only screens (loses Arta ADR-0001 — real React screens as files);
   reimplementing compile-on-demand natively (months of work duplicating Vite).
2. **Entry point = Rail** (far-left 56px column, currently sparse), one button
   per view, plus keyboard shortcuts ⌘D (Design) / ⌘⇧A (Artifacts). REJECTED:
   Roster footer menu (human forbade), agent tab strip (conflates agent tabs
   with view tabs).
3. **Artifact ingestion = CLI verb + DB.** `conclave artifact add/list/get`
   writes the existing (extended) `artifact` table; engine emits
   `artifact:changed`; any CLI vendor can call the conclave binary. REJECTED:
   watched `artifacts/` folder (no metadata: title/creator/kind); both paths
   (two code paths to maintain — can add the folder fallback later if a
   non-conclave-aware CLI shows up).
4. **Embed the WHOLE Arta viewer in the iframe** (its own tab nav: Prototype /
   Data / Flow / Architecture / Plan). Conclave does NOT re-render the document
   tabs natively. REJECTED: porting tab components into Conclave's React tree
   (~5k LOC duplicated, two design systems to keep in sync). Revisit only if
   iframe UX proves insufficient.

## Architecture

```
agent CLI (any vendor)                      Conclave app
  │  writes files                            ┌──────────────────────────────┐
  ▼                                          │ AppShell                     │
<workspace folder>/.arta/                    │  Rail: [Design] [Artifacts]  │
  state.json  proto/*.tsx                    │  Design view:                │
  feedback.json  runtime.json                │   ┌────────────┬───────────┐ │
  ▲            ▲                             │   │ iframe     │ terminal  │ │
  │ chokidar   │ HTTP /__arta/*              │   │ (viewer)   │ (kept     │ │
  ▼            ▼                             │   │            │  mounted) │ │
design-viewer sidecar (Vite, node)  ◄────────┼───┴────────────┴───────────┘ │
  spawned + health-checked by Rust engine    └──────────────────────────────┘

conclave artifact add ──► engine DB (artifact table) ──► artifact:changed ──► Artifacts view
```

- **The `.arta/` file contract is the vendor-neutral API.** No MCP anywhere.
  Agents write `state.json` and `proto/*.tsx` with their ordinary file tools;
  they read `feedback.json`/`runtime.json` the same way. A new bundled skill
  (`design-canvas`) teaches the contract.
- The sidecar is Arta's viewer with the vendor coupling stripped: no MCP
  server, no `.claude-plugin/`, registry moved from `~/.arta/registry.json`
  to Conclave's app-data dir, port default **7343** (not 7317, so a standalone
  Arta can coexist), binds 127.0.0.1 only.

## Lanes

### Phase 1 (parallel, disjoint boundaries)

**Lane A — `artifact-store`** (implementer: Tiësto · reviewer: Armin)
Rust engine + CLI + ipc surface for artifacts. See task plan for full detail.
- Migration `0014_artifact_workspace.sql`: extend `artifact` with
  `workspace_id TEXT`, `agent_id TEXT`, `title TEXT`, `kind TEXT`,
  `content TEXT`; make `message_id` nullable (SQLite: table rebuild).
  Existing chat-parsed rows keep working.
- `repo/artifact.rs` + `commands/artifact.rs`: `artifact.add/list/get`
  ipc commands; emit `artifact:changed { workspaceId }`.
- `conclave-cli`: `artifact add <ws> --title <t> --kind <k> (--file <p> |
  --content <text>)` → prints id · `artifact list <ws>` · `artifact get <id>`.
  Kinds: `markdown|code|html|svg|mermaid|react|text`.
- Frontend surface only (no view): extend `Artifact` type in `src/ipc/types.ts`,
  add commands in `commands.ts`, event + hook in `events.ts`.
- Bundled skill snippet: artifact criteria (>15 lines, self-contained,
  reusable) added to the tool-map skill table + a short Artifacts section.

**Lane B — `design-sidecar`** (implementer: Dew · reviewer: Mellow)
Vendor the viewer, manage its lifecycle, expose ipc. See task plan.
- `design-viewer/`: vendored subset of `/Users/detoro/code/arta` —
  `src/`, `vite/`, `proto/`, `bin/`, `vite.config.ts`, `package.json`
  (own package, pnpm). STRIP: `mcp/`, `.claude-plugin/`, `skills/`,
  `commands/`, `evals/`, brust/pdf export, puppeteer headless capture
  (client-side `modern-screenshot` capture stays). Registry path →
  `<app-data>/design-viewer/registry.json`; port env `CONCLAVE_DESIGN_PORT`
  default 7343; remove all `CLAUDE_*`/`ARTA_DIR` env names
  (`CONCLAVE_DESIGN_*`).
- `runtime/design_viewer.rs`: spawn `node design-viewer/bin/viewer.mjs`
  (resolve node from PATH like agent CLIs; on missing node → typed error the
  view can render), health check `GET /__arta/state`, restart-on-crash with
  backoff, kill on app exit, one sidecar serves all workspaces.
- `commands/design.rs`: ipc `design.ensure { workspaceId }` →
  `{ url, port, projectId, running }` — scaffolds `.arta/` in the workspace
  linked folder if missing (reuse arta's `scaffold.ts` templates), registers
  the project, ensures sidecar up. `design.status { workspaceId }` same shape,
  no side effects.
- Bundled skill `design-canvas/SKILL.md`: the `.arta` file contract for agents
  (state.json schema pointer, proto screen conventions, feedback/runtime
  loop) — vendor-neutral wording, no MCP references.

### Phase 2 (cut after Phase 1 merges; frontend)

**Lane C — `artifacts-view`**: Rail buttons (BOTH views' shell wiring lands
here: two AppShell flags + two Rail buttons + ⌘D/⌘⇧A + native menu items),
`ArtifactsView.tsx` full-page gallery (replace-pane pattern is fine — no
terminal needed): list from `ipc.artifact.list`, live via `artifact:changed`,
render via existing `ArtifactFrame`/`withSandboxCsp` (Preview/Code toggle),
markdown/code/svg renderers per kind. Candidate: Dabin.

**Lane D — `design-view`**: `DesignView.tsx` split layout INSIDE
WorkspacePane's tree — CRITICAL: keep `<Terminal key={sid}>` mounted. The
tree shape must be identical in both modes (left slot width 0 ↔ flex-1
iframe; main column flex-1 ↔ w-[420px]) so React never remounts the terminal
(see `Terminal.tsx:16-30` scrollback-loss note). Iframe src from
`ipc.design.ensure`; error state when node missing. Candidate: Tiësto or Dew.

Lane C lands before Lane D (C owns the shared shell wiring; D consumes it).

## Global constraints (every lane inherits)

- App UI copy is **English** (recorded memory: conclave-ui-copy-english).
- Design canon: existing Conclave patterns — MemoryGraph/LaneBoard header
  pattern, `app.css` tokens, lucide icons. The embedded viewer keeps Arta's
  own design (it lives in the iframe). No new visual language.
- Gates per lane: `cargo test` + `cargo clippy --all-targets -- -D warnings`
  (Rust); `pnpm build` incl. `tsc` (frontend). Commit first, then
  `conclave task gate` (gate pins HEAD).
- Lane worktrees via `conclave lane start`; lead integrates; nobody merges
  their own lane.
- **Shared wiring files** (AMENDED 2026-07-05 per challenge d34d6393 on
  design-artifact-store, found by Tiësto, verified by Armin — the original
  plan named only mod.rs/commands.ts and was wrong): any lane that adds a
  command, event, CLI verb, or migration ALSO touches these choke points, and
  a lane plan must name them in its boundary by default:
  - `src-tauri/src/engine/router.rs` — dispatcher match arm (ipc AND cli.exec
    both route through it)
  - `src-tauri/src/engine/commands/cli.rs` — `map_argv` allowlist arm (the
    no-passthrough security choke point)
  - `src-tauri/src/engine/bus.rs` — event-name constant + payload struct
  - `src-tauri/src/engine/db.rs` — `migrate()` needs an explicit
    `if version < N` block per migration; a .sql file alone NEVER runs
  - `src-tauri/src/engine/{commands,repo,runtime}/mod.rs`, and
    `src/ipc/{commands,events,types}.ts`
  Edits to all of these stay one-arm/one-line additive; lead resolves merge
  collisions at integration. Lane A's contested set (db.rs, router.rs,
  commands/cli.rs, bus.rs) and Lane B's (router.rs, db.rs if 0015 exists)
  land per the immutable-boundary protocol: ruling on the ledger + this
  amendment = the boundary record; the files land as a separate raw git
  commit in the lane worktree with explicit pathspec and
  `--author "Name <agentId@agents.conclave.local>"`.

## Risk ledger

- **Prod packaging of the sidecar** (node_modules ~hundreds of MB) is OUT of
  Phase 1 scope: dev-mode resolves `design-viewer/` from the repo/app dir and
  runs `pnpm install` on first ensure (into the vendored dir). A follow-up
  task will decide bundling (Tauri resources vs first-run install into
  app-data). Do not block on it.
- Node not on PATH → `design.ensure` must return a typed error, not hang;
  Design view renders a friendly English message.
- Port 7343 in use → sidecar takes next free port; the ipc response carries
  the real URL (frontend never hardcodes).
- Arta's `/@fs/` imports need the workspace dirs allowed in the vendored
  `vite.config.ts` `server.fs.allow` — arta already handles this; verify when
  vendoring.
- Two React copies inside the iframe would crash hooks — keep arta's
  `dedupe`/`alias` config intact when vendoring.
- xterm remount modes: verify Design view toggle in BOTH `remount` and
  `keep-alive` term modes (`src/lib/termMode.ts`).
- `.arta/` scaffolded into a user's linked folder that is a git repo — that is
  by design (same as standalone Arta); the agent/human decide whether to
  commit it.

## Records

- This plan: `docs/2026-07-05-plan-design-artifacts-views.md` (master).
- Task objects: `design-artifact-store` (Lane A), `design-sidecar` (Lane B);
  Phase 2 tasks cut after Phase 1 merges.
- Source exploration reports live in the lead's session; the load-bearing
  facts are restated here — implementers need only this doc + task plans.

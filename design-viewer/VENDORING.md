# Vendoring notes

This package is a vendored subset of the Arta viewer (`/Users/detoro/code/arta`,
commit at time of vendoring), stripped of every CLI-vendor coupling per
`docs/2026-07-05-plan-design-artifacts-views.md`. Re-syncing from upstream
later must re-apply every deviation below.

## Cut (not copied)

- `mcp/`, `.claude-plugin/`, `skills/`, `commands/`, `evals/`, `harness-eval/` —
  the whole Claude-plugin / MCP surface. **Exception:** `mcp/slop-detect.mjs`
  (a self-contained, dependency-free JS module with zero MCP coupling) is
  vendored to `vite/slop-detect.mjs` — it is load-bearing for `arta-watch.ts`'s
  watch-time slop gates (ADR-0004), which we DO keep. Its own test file
  (`slop-detect.test.mjs`) was not copied.
- `vite/export-brust.ts`, `vite/export-pdf.ts`, `vite/export-static.ts`,
  `vite/headless-snapshot.ts` and every `*.test.ts` under `vite/`.
- The `/__arta/export`, `/__arta/export-pdf`, `/__arta/capture`, and
  `/__arta/exports/*` routes in `arta-watch.ts`, and their UI (PrototypeTab's
  PDF/HTML export buttons + result modal). Client-side snapshot capture
  (`src/lib/capture.ts`, `modern-screenshot`, the `/__arta/snapshot` POST
  route) stays — it never used Puppeteer.
- Deps dropped from `package.json`: `@modelcontextprotocol/sdk`, `jspdf`,
  `puppeteer-core`, `zod` (only used by the stripped `mcp/` code), `@types/bun`
  (bun-only dev dep, no longer building an mcp bundle with bun).
- `bin/arta.mjs` is replaced by `bin/viewer.mjs` (below), not copied.

## Renamed / repointed

- Registry file: `~/.arta/registry.json` → `$CONCLAVE_DESIGN_HOME/registry.json`,
  falling back to `~/.conclave/design-viewer/registry.json` (`vite/projects.ts`).
  The FNV-1a `idFor()` hash is byte-for-byte UNCHANGED — the Rust engine
  (`engine/runtime/design_viewer.rs`) reimplements it and the two must agree
  (cross-language `cargo test`).
- Port: hardcoded `7317` → `$CONCLAVE_DESIGN_PORT`, default **7343**
  (`vite.config.ts`, `bin/viewer.mjs`).
- Bind address: `0.0.0.0` + `allowedHosts: true` → `127.0.0.1` only
  (`vite.config.ts`) — this sidecar is never meant to be LAN/tunnel-reachable.
- Version display (`__ARTA_VERSION__`, Topbar): read from
  `.claude-plugin/plugin.json` → read from this package's own `package.json`
  (`vite.config.ts`).
- Package identity: `arta` → `conclave-design-viewer`; `bin.arta`/`bin.arta-mcp`
  → `bin.conclave-design-viewer` (`bin/viewer.mjs`); scripts dropped
  `build:mcp`/`mcp`/`eval:gate`.

## Removed env reads

- Every `ARTA_DIR` / `CLAUDE_*` read is gone (`arta-watch.ts`, `proto-app.ts`).
  Upstream used `ARTA_DIR` to point one launched viewer at ONE project's
  `.arta/` (the "home" project, always present even before its first write).
  This sidecar has no such concept — **one process serves every Conclave
  workspace**, each registered into `registry.json` by the engine's
  `design.ensure` ipc command. Consequences, both in `arta-watch.ts`:
  - `homeDir` now always resolves to `<design-viewer-root>/.arta` — a
    directory that structurally never exists in this deployment.
  - `loadProjects()`'s `add(homeDir, ..., requireState)` call was changed from
    `requireState: false` to `true` (matching every other registry entry) so
    this phantom project never appears in the project switcher. Upstream
    needed `false` so a brand-new, not-yet-written project still showed up;
    here that project simply never exists.
  - The home-project auto-scaffold bootstrap in `configureServer` was removed
    entirely (dead code once there's no real home project) — every real
    project's `.arta/` is scaffolded by the engine's `design.ensure` command
    before this sidecar is ever asked to start.
- `bin/viewer.mjs` (new, replaces `bin/arta.mjs`): no bun-install-and-retry
  fallback (the engine's supervisor runs `pnpm install`), no
  process-matching/GC of other viewer instances (the engine owns lifecycle:
  one supervised child, crash-restart with backoff, killed on app exit).
  Prints one ready line to stdout, `DESIGN_VIEWER_READY port=<p>`, which the
  Rust supervisor scans for.

## UI copy fixed (not just cut)

- `PrototypeTab.tsx`'s post-feedback instructions told the dev to run
  `/arta:arta feedback` — a Claude-plugin slash command that does not exist in
  Conclave. Changed to point at `.arta/feedback.json` directly (any agent
  reads it per the `design-canvas` skill), since leaving the old copy would
  actively mislead rather than merely look different.

## Kept intact (ADR-backed, do not touch without a challenge)

- The `/@fs/` virtual-module manifest generation (`vite/proto-app.ts`,
  `vite/proto-manifest.ts`).
- HMR WS push events (`arta:update` / `arta:change` / `arta:projects`).
- The iframe sandbox + postMessage bridge (`proto/shell/`).
- `theme.css` Thai-font fallback (`injectThaiFallback`, `proto-app.ts`).
- The curated-dependency dedupe/alias in `vite.config.ts` (two React copies in
  one page crashes hooks — ADR-0002).

## Security posture deviation from the risk ledger (`server.fs.allow` → `fs.strict: false`)

The plan's risk ledger called for a SCOPED `server.fs.allow` admitting only
registered project dirs. Upstream Arta has no such scoping (checked
`vite.config.ts` and `mcp/server.mjs` — no `fs.allow` logic exists anywhere),
so the smoke check hit Vite's DEFAULT allow-list (this package's own root
only): every `/@fs/` screen import from a real registered project 403'd.

Implemented instead (`vite.config.ts`): `server.fs.strict: false`, which
disables the check entirely rather than scoping it. **This is broader than
the plan intended and carries a real Phase-1 risk**, flagged in review
(Mellow, challenge `4d81be26`): with `fs.strict: false`, this loopback Vite
server will serve ANY absolute path via `GET /@fs/<abspath>` to ANY same-machine
requester — not just this engine's own registry.json contents. Binding to
`127.0.0.1` blocks LAN access but not another local process, nor a webpage
the user's browser has open (the classic Vite dev-server localhost-file-read
/ DNS-rebind vector — e.g. a malicious page could attempt to fetch
`http://127.0.0.1:7343/@fs/Users/<you>/.ssh/id_rsa` while the sidecar runs).

Accepted for Phase 1 as an explicit dev-mode threat-model call (loopback-only
sidecar, developer's own machine) rather than implemented as a real
`fs.allow` scope — a correctly scoped allow-list would need to grow dynamically
as workspaces register (unlike upstream's single fixed project dir), which is
a small enough follow-up to track rather than block this lane on. A
Host/Origin-header guard (and/or `server.fs.deny` for secret-like globs) is
the candidate hardening; tracked as a follow-up, not implemented here.

## Smoke check

```
cd design-viewer && pnpm install
CONCLAVE_DESIGN_HOME=/tmp/conclave-design-smoke node bin/viewer.mjs --port 7343
# then: register a scratch project dir into
# /tmp/conclave-design-smoke/registry.json, open
# http://127.0.0.1:7343/?project=<id>, confirm state renders + a proto screen compiles.
```

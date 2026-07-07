# design-host: per-workspace libs + public assets — design spec

Date: 2026-07-07 · Owner: Detoro (bfb737ff, lead) · Status: approved by human (chat, 2026-07-07)

## Mission

The design system is now in real use outside this repo (first deployment:
`/Users/detoro/code/ket-doc/design` — a KetDoc workspace with `screens/`,
`components/`, `lib/`, `theme.css`). Two capabilities are missing for real
work:

1. **Extra libraries.** Screens can only import the 8 CURATED packages
   (react, react-dom, react-router-dom, motion, lucide-react, recharts,
   clsx, tailwind-merge) aliased to the host's own `node_modules`
   (`design-host/vite.config.ts`). A workspace cannot add e.g.
   `@tanstack/react-table` today.
2. **Static assets.** No supported way to use images/fonts/files from a
   screen — no assets convention, no public dir.

## Decisions (settled with the human, 2026-07-07)

### D1 — Per-workspace dependencies, resolved explicitly

- A workspace MAY have `design/package.json`; anything it installs lands in
  `design/node_modules` and is importable from that workspace's screens,
  components, and lib only.
- `designApp()` (`design-host/vite/host-app.ts`) grows a `resolveId` hook:
  for a **bare** import whose importer file lives under a registered project
  dir (`vite/projects.ts` registry) and whose package name is NOT in the
  CURATED set, resolve it from that workspace's `design/node_modules`
  (Node resolution rooted at the workspace's `design/` dir) and return the
  absolute file path (Vite serves it via `/@fs/`).
- CURATED specifiers keep hitting the existing `resolve.alias` FIRST, always
  — a workspace that accidentally installs its own `react` still gets the
  host's single copy. No dual-React/hook crashes by construction.
- A bare import that resolves nowhere fails with an actionable overlay
  error: `"<pkg>" is not installed in this workspace — add it to
  design/package.json` (never a silent 500 or a bare Vite resolve error).

**Rejected alternatives** (recorded so they are not re-proposed):

- *Rely on Vite's natural node-walk resolution from the `/@fs/` importer* —
  behavior outside the Vite root is undocumented/undeterministic and yields
  unhelpful errors; keep resolution explicit and loggable.
- *Extend the central CURATED set per request* — leaks versions across
  workspaces, requires touching the host for every new lib (human rejected).

### D2 — Host auto-installs workspace deps

- The **host process** (Node side, inside `designApp()`), not the Rust
  engine, owns installs: it is long-lived, already has the registry, and
  already watches files.
- On project registration (first manifest load) and on any
  `design/package.json` add/change: if `node_modules` is missing or stale
  (package.json mtime newer than the install marker), run an install in the
  workspace's `design/` dir, then send a `full-reload`.
- Package-manager choice follows the `design-host-node-guard` ruling
  (merged 9f11f98): pnpm only if **workable** (`pnpm --version` exits 0 and
  parses), else npm. Presence on PATH is not workability.
- Installs are serialized per workspace (no concurrent double-install) and
  failures surface in the host log AND as an overlay-visible error — never
  swallowed.

### D3 — Assets, both models

- **Relative import (primary, idiomatic):** files under `design/assets/`
  imported relatively from screens/components (`import logo from
  "../assets/logo.png"`) and via `url()` in `theme.css`. Expected to ride
  Vite's existing asset pipeline through `/@fs/`; the plan MUST verify this
  end-to-end (pixel gate) and document it rather than assume it.
- **Public dir (absolute URLs, mirrors a real project's `public/`):**
  `design/public/` served at `/p/<projectId>/<path>` by a new middleware in
  `designApp()`.
  - Path-traversal guard: resolved target must stay inside
    `design/public/`; otherwise 403. Unknown project id or missing file →
    404. Correct Content-Type; `Cache-Control: no-store` (design iteration,
    not production).
  - So screens never hard-code a project id: the app entry exposes
    `window.__DESIGN_PUBLIC_BASE__` (`"/p/<projectId>"`) before any screen
    loads; workspaces may wrap it in a tiny `lib/` helper
    (`asset("logo.png")`).

### D4 — Teach the contract

- Update `design-host/README`-level docs and the `design-craft` skill(s):
  how to add a lib, both asset models, the CURATED-set override rule, and
  the ESM caveat (below). One import contract, taught once.

## Error handling

- Unresolvable bare import → named, actionable overlay error (D1).
- Install failure → logged with the manager's stderr tail; the manifest
  still loads (screens that don't need the new dep keep rendering).
- Public middleware → 403 traversal, 404 unknown, never throws the server.

## Testing / acceptance

- Unit: resolver (curated vs workspace vs missing), staleness check,
  traversal guard.
- End-to-end on the ket-doc workspace (the real deployment):
  1. add an ESM lib to `design/package.json` → host installs, screen
     imports and renders it;
  2. image via relative import renders;
  3. file in `design/public/` renders via `__DESIGN_PUBLIC_BASE__` URL;
  4. existing workspaces without `design/package.json` behave exactly as
     today (no install attempt, no behavior change).
- UI pixel gate per CLAUDE.md standing protocol for anything touching
  rendered output.

## Known limitation (accepted)

- **ESM packages are the supported target.** CJS-only packages are not
  prebundled for workspace deps (Vite only prebundles its own root), so
  they may fail; the failure must be a clear overlay message suggesting an
  ESM build/alternative. If the implementation finds a cheap per-workspace
  prebundle, good — but it is not required for v1.

## Out of scope

- Bundling a Node runtime; auto-repairing user toolchains (unchanged from
  the packaging/node-guard lanes).
- Lockfile policy across machines; native-build (node-gyp) deps.
- Production build/export of screens — this is the dev canvas only.

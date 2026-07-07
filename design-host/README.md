# design-host

The Node-side Vite dev server that renders a workspace's `design/` folder as a
live canvas (spawned and supervised by the Rust engine, `runtime/design_host.rs`).
One host process serves every registered workspace via the shared project
registry (`vite/projects.ts`). See:

- `docs/superpowers/specs/2026-07-05-uishot-real-pixels-design.md` — the
  original design-canvas host design.
- `docs/superpowers/specs/2026-07-07-design-host-workspace-libs-assets-design.md`
  — per-workspace libraries and static assets (this doc's subject).

## Adding libraries

A workspace MAY declare its own npm dependencies in `design/package.json`.
Anything installed there is importable from that workspace's screens,
components, and lib — and from that workspace **only**.

- **The curated 8 always come from the host** — `react`, `react-dom`,
  `react-router-dom`, `motion`, `lucide-react`, `recharts`, `clsx`,
  `tailwind-merge` (single source of truth: `vite/curated.ts`). Never add these
  to your own `design/package.json` — the host aliases them to its own single
  copy so two React instances never load into one page; a workspace copy is
  simply ignored.
- **Auto-install.** The host watches each registered workspace's
  `design/package.json`. On first registration and on every add/change, if
  `design/node_modules` is missing or stale it runs an install in that
  workspace's `design/` dir, then reloads the canvas. Package manager choice:
  pnpm if it is actually **workable** (`pnpm --version` exits 0 and prints a
  parseable version — a broken shim on PATH does not count), otherwise npm.
  Installs are serialized per workspace; a failing install is logged (with the
  manager's stderr tail) and does not crash the host — screens that don't need
  the new dependency keep rendering.
- **ESM packages are the supported target.** CJS-only packages are not
  prebundled for workspace dependencies, so they may fail in-browser with a
  `require is not defined`-style error. Pick an ESM build/alternative.
- **Missing-dependency error.** A bare import that isn't curated and isn't
  found in the workspace's `design/node_modules` fails with an actionable
  overlay error rather than a bare Vite resolve error:

  ```
  [design-host] "<pkg>" is not installed in this workspace — add it to
  <workspace>/design/package.json and it will be installed automatically
  (ESM packages are the supported target).
  ```

## Assets

Two models, matching how a real project handles static files:

- **Relative import (primary, idiomatic).** Put files under
  `design/assets/` and import them relatively — `import logo from
  "../assets/logo.png"` from a screen/component, or `url(...)` from
  `design/theme.css`. This rides Vite's ordinary asset pipeline through
  `/@fs/`.
- **Public dir (absolute URLs).** Put files under `design/public/`; the host
  serves them at `/p/<projectId>/<path>`. Screens never need to know their own
  project id — before any screen loads, the app entry sets
  `window.__DESIGN_PUBLIC_BASE__` to `"/p/<projectId>"`. A workspace typically
  wraps this in a one-line `design/lib/` helper:

  ```ts
  export const asset = (path: string) => `${window.__DESIGN_PUBLIC_BASE__}/${path}`;
  ```

  then `<img src={asset("logo.png")} />`.

## Caveats

- CJS-only packages may not work — pick an ESM build (see above).
- Public-dir responses are sent with `Cache-Control: no-store` — this is the
  dev canvas, not a production asset pipeline; nothing is meant to be cached
  across restarts.
- Path traversal into `design/public/` is rejected (403); unknown project id
  or missing file is a 404.

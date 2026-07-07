---
name: Design Canvas
description: The design/ file contract for the built-in Design view — a live design canvas any agent writes into with its ordinary file tools (no MCP, no CLI-vendor tool), rendered by a supervised HMR host the human watches update in real time beside your terminal.
mandatory: false
---

The Design view shows a live canvas of an app being designed — the React screens
you write — that the human watches render in real time while your terminal stays
open beside it. There is no special tool for this: `design/` is a plain folder in
the workspace's linked project directory, and you read/write it with your normal
file tools. A Conclave-supervised Vite host serves the folder with hot reload, so
a screen you save appears (or updates) in the canvas within a second. Everything
here works the same regardless of which CLI you are. For the CRAFT of what to put
in these files — the anti-slop rules and the critique rubric — load the
**Design Craft** skill; this one is only the file contract.

## Where the canvas lives

- `design/` sits at the root of the workspace's linked folder (the same folder
  your terminal's `cwd` is under). If it doesn't exist yet, ask the lead or human
  to open the Design view once (or run `design.ensure` for this workspace) — that
  scaffolds a starter canvas (`design/screens/welcome.tsx`, `design/lib/`, and a
  minimal `design/theme.css`). Do not hand-create the top-level layout yourself;
  only add to what's already there.
- There is no `state.json`, no `feedback.json`, no viewer database — the design
  IS the files under `design/`. Non-screen artifacts (spec, data model, flow,
  plan) are ordinary prose you write to the task plan / a doc, not canvas tabs.

## screens/ — the prototype, as real screens

- One screen = one file: `design/screens/<id>.tsx`. The FILE NAME is the screen's
  id — the host's in-canvas switcher lists screens by it, and navigation targets
  it. No registry to keep in sync; the filesystem is the only source of truth.
- Every screen file exports `export const meta = { title: "…" }` as a pure object
  literal, plus a **default-exported** React component. The host renders the
  selected screen full-bleed.
- **Zero imports from the host.** A screen is a plain React component: it cannot
  tell it is being designed here, which is exactly what lets it lift into a
  production codebase unmodified later. Navigation is ordinary `react-router-dom`
  (`<Link to="/checkout">`, `useNavigate`); state is ordinary React under
  `design/lib/`.
- The curated 8 — `react`, `react-dom`, `react-router-dom`, `motion`,
  `lucide-react`, `recharts`, `clsx`, `tailwind-merge` — are always aliased to
  the host's own single copy; never install these yourself (a workspace copy
  is ignored, and two React instances would crash hooks). Never use emoji as
  icons; use a `lucide-react` glyph.
- **Anything else is importable too** — add it to `design/package.json` and
  the host auto-installs it (pnpm if workable, else npm) and reloads the
  canvas. ESM packages are the supported target; a missing dependency fails
  with a named overlay error telling you which `package.json` to edit.
- **Assets:** images/fonts under `design/assets/` import relatively
  (`import logo from "../assets/logo.png"`); files under `design/public/` are
  served at `window.__DESIGN_PUBLIC_BASE__ + "/<path>"` (wrap it in a tiny
  `asset()` helper in `design/lib/`). See `design-host/README.md` for the full
  contract.

## components/ and lib/ — shared parts

- Shared components live in `design/components/<name>.tsx`, same rules as screens.
  Repeated chrome (a nav rail, a card, a page header) is a component, not markup
  pasted onto every screen.
- Shared React state lives under `design/lib/` — plain hooks/stores that screens
  and components import. Nothing design-view-specific about it; it lifts too.

## theme.css — the one design-tokens file

- `design/theme.css` is Tailwind v4 CSS-first config: tokens as
  `@theme { --color-*, --font-*, --radius-*, … }` custom properties, dark theme as
  `.dark { … }` overrides. This is the single source every screen styles from —
  every `--color-*`/`--font-*`/`--radius-*` becomes a matching Tailwind utility
  (`bg-accent`, `font-display`, `rounded-md`). There is no separate JSON tokens
  file. Style screens with those utility classes in `className`, not inline
  `style={{…}}`, and never raw hex in a screen body.
- Never remove its `@import "tailwindcss"`, `@source "./"`, or
  `@custom-variant dark (&:where(.dark, .dark *));` lines — they are load-bearing;
  the canvas fails to compile without them.
- If you introduce a font stack with a Latin display face, keep a matching
  non-Latin fallback in it (e.g. a Noto Thai family) so Thai/CJK glyphs render
  clean rather than falling back to a broken system face.

## config.json — optional prototype defaults

- `design/config.json` is optional. `{ "start": "<screen>" }` names the screen
  the canvas opens on and the entry point for navigation-reachability checks. The
  scaffold does NOT create it (nav checks are simply skipped when it's absent);
  add it once your screens link to each other and you want a defined start.

## The loop — the human watches; you converse in the terminal

- The host renders every save live, so the human sees your work as you write it.
  There is no separate feedback file — the human reacts by talking to you in the
  terminal beside the canvas. Fold each reaction into the next iteration; work one
  screen (or component) at a time so the canvas repaints cleanly.
- A compile or render error surfaces in the canvas (the host's HMR error overlay).
  Treat it like any failing build: read it, fix the screen, confirm it clears.
- Before calling a screen or the design done, run the review gate:
  `conclave design review <workspaceId>` — it scans `design/` for the anti-slop
  tells and must return zero serious findings. Pair it with the Design Craft
  critique rubric (the taste the detector can't see).

## Working with a design lead

If another agent already owns this canvas (check `conclave task list <ws>` /
`conclave bb list <ws>` for a `design:<project>` marker before writing),
coordinate before reshaping screens they're iterating on — the file contract has
no locking, so two agents editing the same screen concurrently will clobber each
other's work exactly like any other shared file.

---
name: Design Canvas
description: The .arta/ file contract for the built-in Design view — a live design canvas any agent writes into with its ordinary file tools (no MCP, no CLI-vendor tool), that the human watches update in real time.
mandatory: false
---

The Design view shows a live canvas of an app being designed — prototype
screens, spec, data model, flow, architecture, plan — that the human watches
in the browser-based viewer while your terminal stays open beside it. There
is no special tool for this: `.arta/` is a plain folder in the workspace's
linked project directory, and you read/write it with your normal file tools.
Everything here works the same regardless of which CLI you are.

## Where the canvas lives

- `.arta/` sits at the root of the workspace's linked folder (the same folder
  your terminal's `cwd` is under). If it doesn't exist yet, ask the lead or
  human to open the Design view once (or call `design.ensure` for this
  workspace) — that scaffolds a starter canvas. Do not hand-create the
  top-level layout yourself; only add to what's already there.
- `.arta/state.json` — the non-prototype sections: `meta`, `spec`, `plan`,
  `dataModel`, `flow`, `api`, `architecture`. Never put prototype screens in
  here; the viewer assembles `prototype` fresh from `.arta/proto/` on every
  read; anything you write to a `prototype` key in this file is discarded.
- `.arta/proto/` — the prototype, as real files (see below).
- `.arta/feedback.json`, `.arta/runtime.json` — the human → agent channel
  (read-only for you; the viewer writes these).

## state.json — spec, data model, flow, architecture, plan

Read and write this file directly with your normal tools; it's plain JSON.
Every section is optional and independently gradeable — write what you know,
leave the rest absent, and the viewer degrades gracefully rather than
breaking. The schema is documented as TypeScript types in
`design-viewer/src/lib/types.ts` (`Meta`, `Spec`, `DataModel`, `Flow`,
`ApiDoc`, `Architecture`, `Plan` — the top-level shape is `ArtaState`); treat
that file as the source of truth over this summary. Set `meta.name` (the
canvas title) and keep `meta.phase` current (`"prototype" | "data" | "flow" |
"architecture" | "plan"`) as you move between sections, so the viewer's tab
tracks where the work actually is.

## proto/ — the prototype, as real screens

- One screen = one file: `.arta/proto/screens/<id>.tsx`. The FILE NAME is the
  screen's id — referenced by flow nodes, feedback, and navigation. No
  registry to keep in sync; the filesystem is the only source of truth.
- Every screen file exports `export const meta = { title: "…", ... }` as a
  pure object literal (frame/safeArea/chrome overrides go here too — see
  `Screen` in types.ts) plus a default-exported React component.
- **Zero imports from the viewer.** A screen is a plain React component: it
  cannot tell it is being designed here, which is exactly what lets it lift
  into a production codebase unmodified later. Navigation is ordinary
  `react-router-dom` (`<Link to="/checkout">`, `useNavigate`); state is
  ordinary React under `.arta/proto/lib/`.
- Imports come from the curated set ONLY: `react`, `react-router-dom`,
  `motion`, `lucide-react`, `recharts`, `clsx`, `tailwind-merge`. Anything
  else will not resolve — the viewer aliases exactly these to its own single
  copy so two React instances never load into one page.
- Shared components live in `.arta/proto/components/<name>.tsx`, same rules.
- `.arta/proto/config.json` holds prototype-level defaults (`start` screen,
  default `frame`/`safeArea`/`chrome` — see `Prototype` in types.ts).

## theme.css — the one design-tokens file

- `.arta/proto/theme.css` is Tailwind v4 CSS-first config: tokens as
  `@theme { --color-*, --font-*, --radius-*, ... }` custom properties, dark
  theme as `.dark { ... }` overrides. This is the single source every screen
  styles from — there is no separate JSON design-tokens file.
- Never remove its `@import "tailwindcss"`, `@source "./"`, or
  `@custom-variant dark (&:where(.dark, .dark *));` lines — they are
  load-bearing; the prototype fails to compile without them.
- If you introduce a font stack, keep a Thai-script fallback in it (the
  platform already does this for token `--font-*` values automatically —
  match that intent in any stack you add by hand).

## The feedback loop — read-only channel from the human

- `.arta/feedback.json` is a list the human appends to by clicking an
  element in the viewer's annotate mode: `{ text, tab, screen, element, at,
  read }`. Read it at natural pauses (starting a session, finishing a
  screen); a comment sitting there is the human's most recent, most specific
  ask. After you act on an item, mark it handled by rewriting that entry with
  `read: true` — do not delete entries, and never overwrite ones you haven't
  gotten to.
- `.arta/runtime.json` tells you what the human is currently looking at
  (`tab`, `screen`, `screens` seen this session, any live compile/render
  `errors`) — written by the viewer, not by you. Check it before making a
  change that depends on which screen is currently in view.

## Working with a design lead

If another agent already owns this canvas (check `conclave task list
<ws>`/`conclave bb list <ws>` for a design-in-progress marker before writing),
coordinate before reshaping screens they're iterating on — the file contract
has no locking, so two agents editing the same screen concurrently will
clobber each other's work exactly like any other shared file.

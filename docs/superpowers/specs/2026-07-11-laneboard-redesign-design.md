# Lane board redesign — design spec

Date: 2026-07-11 · Owner: Detoro (lead) · Design direction settled with the human.

## Goal

Visual refresh of the Lane board (`laneboard` view) — keep the Kanban structure and
the information it shows; raise the craft. Direction: **refined-minimal
(Linear / Height)** — restrained palette, sharp typographic hierarchy, disciplined
spacing, hairline depth, calm. No new brand; stays inside Conclave's existing dark
design system.

## Problems in the current board (baseline)

Baseline shot: `.shots/laneboard-default.png`. Observed:
1. Cards float on a large empty void — columns have no container, so the dead
   vertical space reads as emptiness, not structure.
2. Flat card hierarchy — slug/title/meta compete at similar weight.
3. Context strip ("0 live · 0 working") is prominent but visually inert.
4. The rightmost column clips at the viewport edge; no "done/merged" terminus is
   visible on the board.

## Scope

IN: the board canvas AND the left agent rail (both restyled to one language).
- Board: `src/components/LaneBoard.tsx` (1351 lines) — columns, cards, context
  strip, top bar, empty states.
- Left agent rail: `src/components/AppShell.tsx` (rail shell) + `src/components/Roster.tsx`
  (agent rows) — restyle to match the board's refined-minimal language.

OUT: no data-model / IPC / behavior change (pure visual + the one new collapsed-column
interaction below). No change to what tasks/columns exist beyond the Merged column.

## Visual canon (approved)

The approved board treatment is the mockup committed at
`docs/superpowers/specs/2026-07-11-laneboard-redesign.mockup.html` (rendered:
`docs/superpowers/specs/2026-07-11-laneboard-redesign.mockup.png`; baseline for
comparison: `docs/superpowers/specs/2026-07-11-laneboard-baseline.png`). Open the
`.html` in a browser to see hover/motion. It is the VISUAL reference for the board.
Note: the mockup uses ad-hoc hex for illustration — the implementation MUST use the
real design tokens (D3 + Constraints), not the mockup's literals.

## Design decisions

### D1 — Bounded lanes (fixes the void)
Each column becomes a **full-height bounded lane panel**: very subtle surface
(`--c-fill-softer` / a low-alpha overlay gradient), `1px solid --c-hairline`,
radius ~14px. Header row (status dot · name · count pill · ghost "+"), then a
scrollable body (`overflow-y:auto`) holding the cards. Empty lane shows a muted
dashed placeholder ("No tasks"), not blank space.

### D2 — Card anatomy (layered)
Order, top→bottom: mono slug (`--c-text-tertiary`, ~10.5px) → title
(`--c-text-primary`, ~13px, weight 550, tight leading) → semantic badge row →
hairline meta row (owner flow avatars `from → to` · file count · age right-aligned,
`--c-text-tertiary`). A 2.5px status-colored left edge (subtle, ~.7 opacity).
Hover: `--c-surface` → raised, border `--c-hairline`→brighter, `translateY(-1px)`.

### D3 — Status + semantic color system (map to REAL tokens)
- Lane status dots: planned → `--c-text-tertiary`; claimed → `--c-accent`;
  in-progress → `--color-status-working` (amber, add a soft glow ring); review →
  `--color-a-violet`; merged → `--c-success`.
- Gate badges: pass → `--c-success` on `--c-success-soft`; fail → `--c-danger` on
  `--c-danger-soft`; challenge → `--c-warning-soft` (amber). SHA in the badge uses
  `--c-text-tertiary`, mono.
- Agent avatars: use the existing `--color-agent-*` palette (atlas/echo/iris/
  maestro/sol/vega) keyed per agent — do NOT hardcode avatar hex.

### D4 — Context strip → live summary
Replace the flat text with: live/working **stat counters** (tabular-nums), a live
dot (`--color-live`) when agents are live, a live-agent **avatar stack**, and a
right-aligned status line ("Tiësto working · N agents live"). Calm/empty state when
no agents are live (no stat glow, muted line).

### D5 — Merged column, collapsed by default
Add a **Merged** terminus column completing planned→…→merged. It renders
**collapsed by default**: a thin vertical rail (~40px) showing the merged count and
a status dot, click to expand to a full lane. Keeps the board uncluttered while the
full flow stays legible. (Collapse state is view-local UI state; no persistence
required for v1.)

### D6 — Left agent rail (restyle to match)
Restyle `AppShell` rail + `Roster` rows in the same language: hairline dividers,
`--c-text-*` hierarchy for name/role/level, the `--color-status-*` working/idle
indicator as a small dot (not the current heavier treatment), avatars from
`--color-agent-*`, and the nav items (Blackboard/Memory/Chat/Lane board) as calm
rows with the active item on `--c-surface`. No structural/behavior change — visual
only, consistent radii/spacing with the board.

### D7 — Top bar
Tighten: segmented Board/Org on `--c-fill-soft`, stat chips (`N tasks`, `N open`
with `--color-status-working` accent), filter field, lane selector — all on
hairlines, calm.

### D8 — Motion (subtle)
Card hover lift + border-brighten (~140ms ease). Lane-collapse expand/collapse
animates width. No gratuitous motion.

## Constraints (inherited — every implementer honors)

- **Design tokens only** — use the `--c-*` / `--color-*` custom properties (see
  `--c-surface|hairline|accent|success(-soft)|warning-soft|danger(-soft)|text-*`,
  `--color-status-*`, `--color-agent-*`, `--color-live`). No new ad-hoc hex.
- **Theme-aware** — must read correctly in both light and dark (tokens flip via the
  `.dark` class; never hardcode a theme's color).
- **English UI copy** (memory: conclave-ui-copy-english).
- **UI Pixel Gate (STANDING)** — `pnpm uishot laneboard` (+ `--scenario empty`) and
  OPEN the PNGs; attach shot paths in the READY note; record via `conclave task gate`.
  Applies to every lane touching `src/`.

## Acceptance

1. `pnpm uishot laneboard` and `--scenario empty` both render, pixel-inspected, and
   visibly match the approved refined-minimal direction (bounded lanes, layered
   cards, live context strip, collapsed Merged, restyled rail).
2. tsc clean; no console errors in uishot.
3. No behavior/data regressions (columns, filters, Board/Org toggle still work).

## Out of scope / deferred

- Column reordering, drag-and-drop between lanes (not in baseline; not added).
- Persisting the Merged-collapse state across reloads (v1 = session-local).
- Any Org-view redesign (this spec is the Board view; Org toggle untouched).

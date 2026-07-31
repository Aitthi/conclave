# Design-host screen switcher redesign — collapsed pill + popover

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro) · authority: in-loop

## Why

Human report (2026-07-31, screenshot): the Design view's screen switcher — a
fixed pill ROW at top-right rendered by `Switcher` in
`design-host/src/Shell.tsx` — grows one pill per screen. With 11 screens it
spans the full canvas width and covers the prototype's own header. The code
itself marks this switcher as "the functional baseline" awaiting a real
design (Lane C canon from `docs/2026-07-05-plan-design-native.md` never
landed).

## Decision (human-picked, final)

The human chose **Option A — single collapsed pill + popover** from three
mocked alternatives (auto-hide edge bar and parent-header dropdown were
REJECTED: the edge bar hides which screen is active; the header dropdown
needs a postMessage bridge and still requires an in-iframe fallback for
"Open in browser"). This section is the design canon for this lane.

```
┌────────────────────────────────────┐
│  (prototype full-bleed, unblocked) │
│                  ┌───────────────┐ │
│                  │ 🔍 filter…    │ │
│                  │ app           │ │
│                  │ ● landing-home│ │
│                  │ login      ⌄  │ │
│                  └───────────────┘ │
│          ┌───────────────────────┐ │
│          │ ‹ landing-home · 8/11 ›│ │
│          └───────────────────────┘ │
└────────────────────────────────────┘
```

## Spec (MUST unless marked otherwise)

All changes live in `design-host/` (the engine-spawned Vite host), NOT the
Conclave app's `src/`. Keep the existing inline-style approach of
`Shell.tsx`; no new dependencies.

### Collapsed pill (replaces the pill row)

- `ids.length <= 1` → render nothing (unchanged behavior).
- One pill, `position: fixed; bottom: 16px; right: 16px; zIndex: 9999`
  (bottom-right occludes least — prototypes put their nav at the top).
- Contents, left to right: prev button `‹` · label button
  `<current-id> · <n>/<N> ⌄` (current id truncated with ellipsis at
  ~200px max-width; n = 1-based index of active, N = ids.length) ·
  next button `›`. Prev/next wrap around.
- Look: `background: rgba(24,24,27,0.92)`, `backdropFilter: blur(8px)`,
  `border: 1px solid rgba(255,255,255,0.08)`, `borderRadius: 999px`,
  subtle shadow, `font: 12px ui-monospace, monospace`, text `#fafafa`.
- Idle fade: when the pointer is not over the control AND the popover is
  closed, opacity eases to ~0.4 after ~2s; back to 1 on hover/focus or
  while the popover is open. `transition: opacity 150ms`.

### Popover (opens from the label button)

- Anchored ABOVE the pill (`bottom: ~56px; right: 16px`), width ~260px,
  `maxHeight: 60vh; overflowY: auto`, same dark theme as the pill,
  `borderRadius: 10px`.
- One row per screen id, monospace 12px; active row marked with `●` and a
  highlight background; hover `rgba(255,255,255,0.06)`. Click → select
  (via existing `onSelect`) + close.
- Filter input pinned at the top **only when `ids.length > 8`**,
  autofocused, placeholder `filter…`, case-insensitive substring match.
  Zero matches → a muted "no match" row.
- Keyboard inside the popover: `Esc` closes; `↑`/`↓` move a highlight;
  `Enter` selects the highlighted row (or the first filtered match when
  nothing is highlighted).
- Click/pointerdown outside the popover closes it.

### Global keyboard switching

- `ArrowLeft`/`ArrowRight` on `document` cycle screens (wrapping) ONLY
  when the popover is closed AND `document.activeElement` is not an
  `input`/`textarea`/`select`/contentEditable — prototype screens may
  contain forms; never steal their keys.

### Selection semantics — DO NOT TOUCH

- Reuse the existing `setActive` in `Shell.tsx` (hash `#/<id>` +
  localStorage `conclave-design-active:<project>`); the `LS_KEY` format and
  `pickInitialScreen`/`parseHashScreen` precedence are pinned by
  `design-host/test/screen-selection.test.mjs` and stay byte-identical.

### Pure helper + tests

- Add `filterScreens(ids: string[], query: string): string[]` to
  `design-host/src/screenSelection.ts` (trim query; empty query → all ids;
  case-insensitive substring). Use it in the popover.
- Extend `design-host/test/screen-selection.test.mjs` (same
  esbuild-transform import pattern already used there) with cases: empty
  query, no match, case-insensitive match, whitespace-only query.

## Files (lane boundary)

- `design-host/src/Shell.tsx` — replace `Switcher` (defined here).
- `design-host/src/screenSelection.ts` — add `filterScreens`.
- `design-host/src/index.css` — only if a keyframe/utility is genuinely
  needed; prefer inline styles.
- `design-host/test/screen-selection.test.mjs` — new cases.

Out of scope: `design-host/vite/**` (registry/HMR machinery),
`src/components/DesignView.tsx` (app-side iframe host is untouched),
`src-tauri/**`.

## Gates (record each via `conclave task gate`)

1. `pnpm -C design-host typecheck`
2. `node --test design-host/test/screen-selection.test.mjs`
3. Pixel proof (UI Pixel Gate spirit — design-host is outside `uishot`'s
   view list, so drive it directly): boot the host against a scratch
   project with ~12 dummy screens — `design-host/test/e2e-workspace.test.mjs`
   shows the boot recipe (registry + `bin/host.mjs`) — then screenshot
   (a) collapsed pill, (b) popover open, (c) filter narrowing the list,
   with `conclave browser open/goto/screenshot` or equivalent, and LOOK at
   each PNG before READY. Attach paths in the READY note.

## Risk ledger

- Fresh lane worktrees have no `node_modules` — run `pnpm install` in
  `design-host/` once before typecheck/tests (known workspace quirk).
- A stale design-host dev server from another checkout may hold the port —
  check `lsof` and kill before trusting a manual screenshot.
- The global arrow-key listener is the riskiest piece: verify typing in a
  text input inside a dummy screen does NOT switch screens.
- The popover must not overflow the viewport bottom (it opens upward for
  exactly this reason).

## Escalation

Design/spec conflicts → challenge on the task, Detoro rules. Implementation
judgment inside this spec (exact paddings, shadow values, animation curve)
is the implementer's — log notable choices as task notes.

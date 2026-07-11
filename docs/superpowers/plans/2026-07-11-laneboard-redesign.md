# Lane board redesign — Implementation Plan

> **For agentic workers:** implement task-by-task, in order (all board tasks share `LaneBoard.tsx`, so this is ONE lane, sequential — not parallel). Steps use checkbox (`- [ ]`) syntax.

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop
implementer: Arta (design specialist) · escalation (design/spec conflicts): Detoro, final

**Goal:** Refined-minimal visual refresh of the `laneboard` view — Kanban kept, craft raised.

**Architecture:** Pure presentational refactor of `LaneBoard.tsx` (board: lanes, cards, context strip, top bar, collapsed Merged column) + a matching restyle of the left agent rail (`AppShell.tsx` + `Roster.tsx`). No data/IPC/behavior change. All color via existing design tokens; verified via the UI Pixel Gate.

**Tech Stack:** React + TypeScript + Tailwind (utility classes over `--c-*`/`--color-*` CSS custom properties), Vite fixture mode + `pnpm uishot` for pixel verification.

## DESIGN CANON (read FIRST, it is the source of truth for every visual decision)

1. Spec: `docs/superpowers/specs/2026-07-11-laneboard-redesign-design.md` (decisions D1–D8 + constraints + acceptance).
2. Approved mockup (visual target): `docs/superpowers/specs/2026-07-11-laneboard-redesign.mockup.html` (open in a browser for hover/motion) + `.mockup.png`. Baseline (before): `…-baseline.png`.
   Pinned canon SHA: **38d8dfa**. Designer/escalation: Detoro.
   The mockup's hex is ILLUSTRATIVE — implement with real tokens per spec D3.

## Global Constraints (every task inherits — verbatim from spec)

- **Design tokens only.** Use `--c-*` / `--color-*` (surface/hairline/accent/success(-soft)/warning-soft/danger(-soft)/text-*, `--color-status-*`, `--color-agent-*`, `--color-live`) via the repo's existing Tailwind token classes (grep neighbors: `bg-surface`, `ring-hair`, `text-text-primary/secondary/tertiary`, `text-accent`, `bg-fill-softer`). NO new ad-hoc hex.
- **Theme-aware.** Correct in BOTH light and dark (tokens flip via `.dark`; never hardcode a theme's color). Verify both if practical.
- **English UI copy** only.
- **No behavior/data change.** Columns, filters, Board/Org toggle, card click targets keep working. Presentational only (plus D5's local collapse state).
- **UI Pixel Gate (STANDING, non-negotiable):** for EACH task that changes the view, run `pnpm uishot laneboard` (and `--scenario empty` where the task affects empty states), then OPEN the `.shots/*.png` with an image-capable read and LOOK. Green exit alone does NOT count. Attach shot paths in notes; record via `conclave task gate <ws> laneboard-redesign -- pnpm uishot laneboard`. Before trusting a shot, `lsof -nP -iTCP:1420 -sTCP:LISTEN` and kill any FOREIGN dev server (only this checkout's server is valid).

## Pre-flight (once, before Task 1)

- [ ] Claim + worktree: `conclave lane start <ws> laneboard-redesign` (creates the worktree + branch).
- [ ] `pnpm install` in the worktree (fresh worktrees have no `node_modules` — memory codeup-lane-worktree-needs-own-pnpm-install).
- [ ] Read `LaneBoard.tsx` fully (1351 lines) — map the existing column/card/context render before editing. Note the current fixture scenario for laneboard in `src/fixtures/scenarios/` so uishot has data.
- [ ] Baseline shot: `pnpm uishot laneboard` → open `.shots/laneboard-default.png`, confirm it matches `…-baseline.png` (you're editing the right view).

---

### Task 1: Bounded lanes + column headers + empty state (spec D1)

**Files:** Modify `src/components/LaneBoard.tsx` (the column/lane render).

- [ ] Wrap each column in a full-height bounded lane panel: subtle surface (`bg-fill-softer` or a low-alpha overlay), `ring-1 ring-hair`/`border-hairline`, radius ~14px; a header row (status dot · name · count pill · ghost "+") and a `overflow-y-auto` scrollable body. Match mockup `.lane`/`.lane-h`/`.lane-b`.
- [ ] Empty lane → muted dashed placeholder ("No tasks"), per mockup `.empty`. Do not leave blank void.
- [ ] Status dot colors per spec D3 (planned `text-tertiary`, claimed `accent`, in-progress `status-working`+glow, review `a-violet`, merged `success`).
- [ ] GATE: `pnpm uishot laneboard` + `--scenario empty`; open BOTH PNGs; confirm lanes are bounded, headers correct, empty state renders. Record gate.
- [ ] Commit (stage commit, boundary `src/components/LaneBoard.tsx`): `feat(laneboard): bounded lane panels + headers + empty state (D1)`.

### Task 2: Card anatomy (spec D2 + D3 badges)

**Files:** Modify `src/components/LaneBoard.tsx` (the card render).

- [ ] Rebuild the card: mono slug (`text-tertiary`) → title (`text-primary`, ~13px/550/tight) → semantic badge row → hairline meta row (owner flow avatars `from → to` · file count · age right-aligned). 2.5px status-colored left edge (~.7 opacity). Match mockup `.card`/`.slug`/`.ttl`/`.badges`/`.meta`.
- [ ] Gate/challenge badges per D3: pass → `success`/`success-soft`; fail → `danger`/`danger-soft`; challenge → `warning-soft`; SHA mono `text-tertiary`.
- [ ] Owner avatars use `--color-agent-*` keyed per agent (grep for the existing avatar-color helper in `Roster.tsx`/`LaneBoard.tsx`; reuse it, do NOT hardcode).
- [ ] Hover: raised surface + brighter border + `translateY(-1px)`, ~140ms (D8).
- [ ] GATE: `pnpm uishot laneboard`; open PNG; confirm card hierarchy, badges use tokens, avatars correct. Record gate.
- [ ] Commit: `feat(laneboard): layered card anatomy + semantic badges (D2/D3)`.

### Task 3: Context strip → live summary (spec D4)

**Files:** Modify `src/components/LaneBoard.tsx` (the context strip).

- [ ] Replace flat "live · working" text with: live/working stat counters (tabular-nums), a `--color-live` dot when agents are live, a live-agent avatar stack, right-aligned status line. Calm/muted when none live. Match mockup `.ctx`.
- [ ] GATE: `pnpm uishot laneboard` (default has live agents) + `--scenario empty` (none live → calm state); open both. Record gate.
- [ ] Commit: `feat(laneboard): live-summary context strip (D4)`.

### Task 4: Merged column, collapsed by default (spec D5)

**Files:** Modify `src/components/LaneBoard.tsx` (+ a `useState` for collapse).

- [ ] Add a Merged terminus lane. Default state = collapsed: a ~40px vertical rail showing the merged count + a `success` dot; click expands to a full lane (view-local `useState`, no persistence). Width animates (D8).
- [ ] Ensure merged tasks actually populate this lane from the existing task data (they exist in state; surface the `merged` status). If the fixture has no merged rows, add 1–2 to the laneboard fixture scenario so the state is exercised (fixed literal timestamps only — CLAUDE.md fixture rule).
- [ ] GATE: `pnpm uishot laneboard`; open PNG; confirm collapsed rail by default. (Expanded state is interaction — verify by reading the code + a manual note; uishot captures the default collapsed state.) Record gate.
- [ ] Commit: `feat(laneboard): collapsed Merged terminus column (D5)`.

### Task 5: Top bar tighten (spec D7)

**Files:** Modify `src/components/LaneBoard.tsx` (the top bar).

- [ ] Tighten: segmented Board/Org on `fill-soft`, stat chips (`N tasks`, `N open` with `status-working` accent), filter field, lane selector — hairlines, calm. Match mockup `.topbar`. Keep all existing controls functional (Board/Org toggle, filter, close).
- [ ] GATE: `pnpm uishot laneboard`; open PNG; confirm top bar + that Board/Org/filter still work (read the handlers). Record gate.
- [ ] Commit: `feat(laneboard): tightened top bar (D7)`.

### Task 6: Left agent rail restyle (spec D6)

**Files:** Modify `src/components/AppShell.tsx` (rail shell) + `src/components/Roster.tsx` (agent rows).

- [ ] Restyle to the board's language: hairline dividers; `text-*` hierarchy for name/role/level; `--color-status-*` working/idle as a small dot; avatars from `--color-agent-*`; nav rows (Blackboard/Memory/Chat/Lane board) calm with active on `surface`. NO structural/behavior change — visual only; keep every existing prop/handler.
- [ ] Verify the rail restyle doesn't regress OTHER views that share `AppShell` (the rail is shared). Spot-check `pnpm uishot home` / `pnpm uishot artifacts` still render (the rail appears there too).
- [ ] GATE: `pnpm uishot laneboard` + `pnpm uishot home` + `pnpm uishot artifacts`; open PNGs; confirm rail restyled + no regression elsewhere. Record gate.
- [ ] Commit: `feat(laneboard): restyle left agent rail to match (D6)`.

### Task 7: Final pass — tsc, theme, acceptance

**Files:** none new (verification + any fixups).

- [ ] `pnpm exec tsc --noEmit` → exit 0. Fix any type errors introduced.
- [ ] Theme check: toggle dark/light (fixture renders `.dark`; if feasible, capture one light shot) — confirm tokens flip correctly, no hardcoded-color artifacts.
- [ ] Full acceptance shots: `pnpm uishot laneboard` + `--scenario empty`; open both; confirm the whole view matches the approved mockup direction and spec acceptance (D1–D8). Attach all shot paths in the READY note.
- [ ] `conclave task state <ws> laneboard-redesign review` + READY note (shot paths, gate ids, tsc result). Do NOT self-merge — Detoro integrates.

## Self-review (plan vs spec)

- Spec coverage: D1→T1, D2/D3→T2, D4→T3, D5→T4, D7→T5, D6→T6, D8 folded into T1/T2/T4, constraints+acceptance→T7 + every task's gate. All covered.
- No new types/functions introduced across tasks (presentational); avatar-color helper is REUSED (T2) not redefined — implementer greps for the existing one.
- No placeholders: each task names exact files + the spec decision it realizes + a concrete pixel gate.

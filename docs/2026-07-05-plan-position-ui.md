# Plan: Position System UI (lane `position-ui`)

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro, lead) · authority: in-loop
implementer: named in the task handoff · design-escalation target: Arta (688719b6-741d-43e1-bc6c-9a2e78d4e21b)
spec: docs/2026-07-05-spec-position-system.md (@093b522 final) — engine P1–P4 already landed (merges d69560b, 6f75b1a, 2974c81); this lane is FRONTEND ONLY.

## Design canon (binding)

Canon commit **b2ac739** — read the `.tsx` sources, NOT the `.arta/snapshots/*.png`:

- `.arta/proto/screens/roster-positions.tsx` — position line on the 266px roster card
- `.arta/proto/screens/builder-position.tsx` — Builder role modal extension (Track/Level/Supervisor)
- `.arta/proto/screens/org-chart.tsx` — indented supervisor tree
- `.arta/proto/screens/escalation-trace.tsx` — challenge routing stepper in Lane board task detail
- `.arta/proto/lib/positions.ts` — chain helpers (`chainUp`, `reportsOf`, cycle-safe) and level model
- `.arta/proto/components/Position.tsx` — shared level/reports-to visual atoms

The proto imports mock data from `.arta/proto/lib/data.ts`; the product version binds the same visuals to real `WorkspaceAgent` data. Visual language reuses existing tokens (theme.css, av/pill/lane-card) — zero new visual system. Any visual deviation you think is needed = design escalation to Arta, filed as a task note prefixed ESCALATION; do not improvise.

## Human-ruled decisions this lane inherits (final, do not re-open)

1. **Org chart placement = `Board|Org` segment control in the Lane board header** (LaneBoard.tsx floating header, near line 234–260). Rail entry REJECTED (crowding); Roster footer item REJECTED (human forbade additions).
2. Org tree is built **client-side** from the roster's `supervisorAgentId` links (spec Q5) — no new backend command.
3. Level NEVER gates any action (spec Q3) — UI shows levels, never disables anything by level.
4. NULL level = explicit "Unranked" state; NULL supervisor = "reports to Human" (crown at tree top). Both are first-class visuals in canon, not empty states.
5. App UI copy is **English**.

## Interfaces (already shipped — consume, do not modify)

- `instance.setPosition` — `src/ipc/commands.ts:158` — req `{ workspaceId /* REQUIRED */, workspaceAgentId, level?: string|null, supervisorAgentId?: string|null }`, res `WorkspaceAgent`. Tri-state: omit key = keep, `null` = clear, string = set. Wrapper: `commands.ts:397` (`setPosition`). Backend validates cycles + workspace match (`set_position_validated`) — the UI's self/descendant disabling is UX, not the safety net.
- `WorkspaceAgent` carries `level?`, `supervisorAgentId?`, `supervisorName?` — `src/ipc/types.ts:140-146`.
- `RosterChangedEvent { workspaceId }` — `src/ipc/events.ts` (~line 122) — emitted after every position write. Open views REFETCH the roster on it (same re-list pattern ArtifactsView uses for `artifact:changed`); never patch state from the payload.
- Roster data: `instance.list` / workspace get (`commands.ts:40,132`) already returns `WorkspaceAgent[]` with the position fields.
- Level ids are exactly `junior|mid|senior|principal` (CLI + backend enforce; see spec §5).

## Tasks (build in this order)

### T1 — `src/lib/positions.ts` (new)
Port the canon `lib/positions.ts` helpers to product types: `chainUp(agentId, roster)` and `reportsOf(agentId, roster)` operating on `WorkspaceAgent[]` (id = `workspaceAgentId`, link = `supervisorAgentId`), cycle-safe via visited set exactly as canon. Level model (LEVELS array, `levelOf`) verbatim from canon. Pure functions, no fetching.

### T2 — `src/components/Position.tsx` (new)
Port canon `components/Position.tsx` atoms (level tag, reports-to chip, ladder) onto product types + theme tokens. Shared by T3–T6.

### T3 — Roster card position line (`src/components/Roster.tsx`)
Per `roster-positions.tsx`: level ladder + track on the 266px card (aside at Roster.tsx:459), reports-to chip at the row edge; explicit Unranked and reports-to-human renderings. Subscribe to `roster:changed` → refetch.

### T4 — Builder position pickers (`src/components/Builder.tsx`)
Per `builder-position.tsx`: extend the role area (~line 192+): Track row read-only (= role), Level 4-segment control + "Clear → Unranked", Supervisor list with self AND descendants disabled (use T1 `chainUp`/`reportsOf` for the disable set), live escalation-chain preview. Submit via `setPosition` with tri-state semantics — only send keys the user changed.

### T5 — Org chart in Lane board (`src/components/LaneBoard.tsx`)
Per `org-chart.tsx`: `Board|Org` segment control in the existing floating header; Org mode renders the vertical indented supervisor tree (client-side from roster), Human crown at top, working/last-activity dot per row (roster already carries activity). Refetch on `roster:changed`.

### T6 — Escalation trace (`src/components/LaneBoard.tsx`)
Per `escalation-trace.tsx`: in the task detail (challenge area, LaneBoard.tsx ~433-527), a routing stepper showing where an open challenge routes: chain from `chainUp(implementer)` / owner per spec §3.1 (LCA(challenger, owner)). Purely presentational — routing itself is engine-side; the UI renders the chain, it does not compute authority.

## Verification gates (commit first, then gate — every gate pins HEAD)

- `conclave task gate <ws> position-ui -- sh -c "pnpm build 2>&1 | tail -3"` (tsc + vite build; the ONLY frontend gate — no frontend test runner exists yet, known deferred small)
- `git diff main -- src/components/LaneBoard.tsx` (and Roster/Builder): every hunk semantic, no formatter sweeps (choke-point semantic-diff guard, memory a6a3dd26)
- Live UI proof is env-blocked for agents (WKWebView, no CDP) → convert to r13 human checklist items per deferred-acceptance ruling 35968ae3. State this in your READY note.

## Risk ledger

- **LaneBoard dev-mock path**: LaneBoard falls back to `laneBoardMock` when `task.list` fails outside Tauri (line ~130). The Org segment must not break this path — roster fetch failing should leave Org mode empty-stated, not crash the board.
- **No RTL/vitest runner**: don't add one in this lane (out of boundary); invariants that want a test get a `// r13-checklist` note instead.
- **`supervisorName` is display-only** — never use it as a key; link by `supervisorAgentId`.
- **Stale roster after position write**: `roster:changed` carries only workspaceId by design — views that don't refetch will show stale chains. Every surface T3–T6 must subscribe.
- **Shared checkout**: commit via `conclave stage commit` only (never plain `git commit`); boundary is exactly the 5 files below. If you discover a genuinely needed file outside it, STOP and escalate — the boundary is immutable (memory: widen via lead's raw scoped commit).

## Boundary

`src/lib/positions.ts, src/components/Position.tsx, src/components/Roster.tsx, src/components/Builder.tsx, src/components/LaneBoard.tsx`

# Roster row: drop the level bar, promote supervisor, trash-icon popover

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
implementer: Dew (60ff2775) · reviewer: Mellow (b3a30e7b) · escalation: Detoro via `task challenge`
base: main @ 8dc6b20 · boundary: `src/components/Roster.tsx` only

## Request (human, 2026-09-05, verbatim intent)

> Fix ui ตรงนี้ เอา bar level ออก แล้วย้าย Supervisor มาไว้แทน
> แก้ dot menu ให้เป็น popover ไม่ต้องมีคำว่า Remove agent ใช้แค่ icon trash

Design canon = the human's two screenshots + this plan (no `.arta` proto exists for the roster):

- `docs/superpowers/plans/assets/2026-09-05-roster-row-before.png` — the sidebar roster as it is now.
- `docs/superpowers/plans/assets/2026-09-05-roster-dot-menu-before.png` — the current inline "✕ Remove agent" row that the ⋯ button expands.

## Decisions (Detoro rulings — final, do not re-open)

1. **Level leaves the roster row entirely.** Row 2 currently renders `PositionLine` = `LevelTag` (pips + "Prin"/"Sr") `·` `TrackIcon` role. The whole `LevelTag` goes, including the short text. Level remains visible in Builder and LaneBoard, which are untouched. Rationale: the human said "เอา bar level ออก"; the level text without the pips is the same widget, and the freed width is what stops "Implemen…" truncating.
2. **Supervisor moves from row 1 to row 2, in the slot the level occupied.** Same `ReportsTo` avatar chip, same clickable button (opens `SupervisorPicker` via `onEditSupervisor`), same `aria-label`. Row 1 becomes: name · provider chip · status dot. Keep `label={false}` (avatar only) so role text keeps its room.
3. **⋯ opens a real popover, not an inline row.** Use the existing primitive `src/components/ui/popover.tsx` (base-ui). Content = ONE icon-only button: lucide `Trash2`, `aria-label="Remove agent"`, `title="Remove agent"`, danger colouring. No text label. Clicking it closes the popover and calls `onRequestRemove(trigger)` exactly as today, so `ConfirmLifecycleDialog` (kind `remove`) still guards the destructive action — nothing is removed without the confirm dialog.
4. **Delete the hand-rolled menu state that base-ui now owns**: the `moreOpen` `useEffect` Escape listener and the `mt-1 … border-t` inline block. base-ui closes on Escape and outside click and returns focus to the trigger. Keep `moreButtonRef` only if `onRequestRemove` still needs the trigger element for the dialog's focus return (it does — pass `PopoverTrigger`'s element or keep the ref on it).
5. **No new files, no Position.tsx edits.** `PositionLine` stays exported and used by Builder/LaneBoard. Roster stops importing `PositionLine` and imports `TrackIcon` (or renders the role span inline) — implementer's call, logged as a note.

## Implementation notes (for a zero-context implementer)

- File: `src/components/Roster.tsx`, component `AgentRow` (~lines 175–390 at base SHA).
  - Row 1 (`{/* Row 1 … */}`): remove the `<button … onEditSupervisor …><ReportsTo …/></button>` block.
  - Row 2 (`{/* Row 2 … */}`): replace `<PositionLine levelId=… track=… compact …/>` with `[supervisor button][·][TrackIcon + role text truncating]`. Reuse the exact classes from the removed row-1 button and from `PositionLine`'s track span (`src/components/Position.tsx:192-245`) so type size/colour match the rest of the sidebar.
  - ⋯ button: wrap in `<Popover open={moreOpen} onOpenChange={setMoreOpen}>`; the button becomes `<PopoverTrigger …>` keeping `aria-label`, `aria-expanded`, `onClick`/`onKeyDown` `stopPropagation` (the row itself is a `role="button"` — every inner control must stop propagation or the row selects on click). `<PopoverContent side="bottom" align="end" sideOffset={4} className="w-auto p-1">` overrides the primitive's `w-72 p-3` default.
  - Popover pattern to copy: `src/components/ContextBars.tsx:695-715`.
- Imports: add `Trash2` from lucide-react; drop `X` only if nothing else in the file uses it (grep first); drop `PositionLine` import, add `TrackIcon`.
- Fixture: `src/fixtures/scenarios/data.ts` already gives every agent a `supervisorAgentId` (Detoro roots to Human), so `ReportsTo` renders both branches without fixture changes.

## Gates (record each with `conclave task gate 11ecf99b-… roster-row-supervisor-trash -- <cmd>`)

1. `pnpm build` — exit 0 (tsc + vite).
2. `pnpm uishot home` and `pnpm uishot home --scenario empty` — exit 0, then **Read both PNGs** (`.shots/home-default.png`, `.shots/home-empty.png`) and confirm by eye: no level pips anywhere in the roster, supervisor chip on row 2 left of the role, role text no longer truncated for "Implementer"/"Researcher" at the default sidebar width.
3. Popover open state cannot be captured by uishot (no click support). Verify it with `conclave browser`: `conclave browser open http://localhost:1420/?fixture=default#view=home`, `conclave browser click` the first `[aria-label^="More actions for"]`, `conclave browser screenshot <path under .shots/>`, then Read that PNG: a small popover under the ⋯ button containing only a trash icon; pressing Escape closes it. Attach the shot path in the READY note. Kill any foreign vite on :1420 first (`lsof -nP -iTCP:1420 -sTCP:LISTEN`, per CLAUDE.md).
4. Human acceptance after rebuild+relaunch (visual): the two screenshots in the canon no longer match; clicking ⋯ shows the trash popover; clicking trash opens the existing confirm dialog.

## Risks

- The row is `role="button"`; a popover portal renders outside the row DOM, so clicks inside the popover do NOT bubble to the row — but the trigger does. Keep `stopPropagation` on the trigger.
- base-ui `PopoverPrimitive.Trigger` renders a `<button>`; do not nest another `<button>` inside it.
- `w-72` default on `PopoverContent` would produce a wide empty box — the `className` override is required, not cosmetic.

## Deferred / out of scope

- Builder and LaneBoard still show level pips (untouched by request).
- Any change to `Position.tsx` or `positions.ts`.

## Outcome

_(implementer fills: commits, gate ids, shot paths, deviations)_

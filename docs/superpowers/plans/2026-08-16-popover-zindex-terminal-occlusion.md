# Popover z-index: xterm canvas occludes popup clicks

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Problem (reported 2026-08-16, human + photos)

The "Save handoff" button in the Compact popover (bottom context bar) is
near-unclickable in the real app: only the bottom-most few pixels register.
Photo evidence: I-beam cursor over the button's middle (terminal hit-testing),
hand cursor only at the bottom edge.

## Root cause (reproduced + verified live, fixture mode, 1280x800)

- `src/components/ui/popover.tsx` puts `z-50` on `PopoverPrimitive.Popup`,
  but Popup is `position: static` → its z-index is IGNORED.
- `PopoverPrimitive.Positioner` is `position: absolute; z-index: auto` → the
  whole popover paints/hit-tests at z-auto in the root stacking context.
- xterm.js layers (`.xterm-link-layer` canvas) are `position: absolute;
  z-index: 2` → they hit-test ABOVE the popup wherever it overlaps the
  terminal area. The canvas is transparent, so the popup is fully VISIBLE but
  clicks land on the terminal. A swallowed pointerdown also counts as an
  outside-press → Base UI closes the popover.
- Verified: `document.elementFromPoint` at the Save-handoff center returned
  `CANVAS.xterm-link-layer`; after setting `z-index: 50` on the Positioner it
  returned the BUTTON, and a real click fired `doCompact` (console showed the
  fixture `snapshot.compact` probe).

## Fix (one edit)

In `src/components/ui/popover.tsx` `PopoverContent`:
- Add `className="z-50"` to `<PopoverPrimitive.Positioner>` (line ~16).
- Remove `z-50` from the Popup's className (single source of truth; Popup is
  static so the class was dead anyway).

## Verification

1. Fixture click-through: open `http://localhost:1420/?fixture=default#view=home`,
   click zap trigger, `elementFromPoint` at Save-handoff center must be the
   button; real click must produce the `[fixture] no handler for
   "snapshot.compact"` console error (= onClick fired).
2. UI pixel gate: `pnpm uishot home`, OPEN the PNG and look; popover styling
   unchanged.
3. `pnpm build` (tsc + vite) green.

## Risk ledger

- Only ContextBars.tsx consumes ui/popover today, but the fix is in the shared
  wrapper — any future popover over the terminal is covered.
- Do NOT z-index the xterm layers down; xterm.css is vendor behavior.
- uishot exits 1 on the `[fixture]` console error by design — run the click
  probe with devtools, not uishot.

# Restore animated "working" WorkLine in the CLI AGENTS rail

**Owner: 4fb2198c (Detoro, lead) · authority: in-loop.** Human override, 2026-07-11:
after the D6 laneboard redesign removed the animated "working" sub-line, the human
wants the **full old WorkLine back** — spinning `LoaderCircle` + "working" label,
slide-in — not a pulse or a bare spinner. (AskUserQuestion answer:
"เอา WorkLine เดิมกลับมาเต็ม".)

## Context / why
D6 (commit `3775974`, "restyle left agent rail (D6)") deleted the `WorkLine`
component from `src/components/Roster.tsx` and moved the working signal onto a
static amber status dot. Human wants the visible motion cue back.

## What to do (small, targeted)
1. **Re-add `WorkLine`** to `src/components/Roster.tsx`. Source of truth to copy
   from: `git show 3775974^:src/components/Roster.tsx` — the `WorkLine` function
   (~lines 129-143) and its render usage `<WorkLine working={entry.working} />`
   (~line 223). Uses `LoaderCircle` from lucide-react (already imported for other
   uses — verify the import).
2. **CSS is already present** — do NOT re-add it. `src/styles/app.css:208-256`
   still defines `.roster-working-slot`, `.is-working`, `.roster-working`,
   `.roster-working-content`, `.roster-working-icon`
   (`animation: roster-working-spin 1.6s linear infinite`), `.roster-working-label`
   (+`::after` dots), and `@keyframes roster-working-spin` / `roster-working-dots`.
   Verify the re-added component's class names match these exactly.
3. **Un-double the signal (dot):** the current status dot (`Roster.tsx` ~224-239)
   carries working via `entry.working ? var(--color-status-working) + glow`.
   With WorkLine restored, revert the dot's `entry.working ?` branches so the dot
   shows the plain `statusColor` again (working is carried by WorkLine, matching
   the pre-D6 division of labor). **Keep** D6's `--color-status-*` token mapping
   for idle/status colors and everything else D6 changed (footer nav, etc.).
4. **Reduced motion (nice-to-have, don't block):** if quick, guard the two
   `@keyframes` with `@media (prefers-reduced-motion: reduce)` in app.css so the
   sub-line still appears but stops spinning. The pre-D6 version had no such guard;
   add it only if trivial.

## UI Pixel Gate (STANDING PROTOCOL — mandatory before READY)
- `pnpm uishot laneboard` — then **open the PNG** (Read the `.shots/*.png`).
- CAVEAT: a static PNG cannot show the spin/slide motion. What the shot MUST
  confirm: for a fixture agent with `working: true`, the "working" sub-line
  (spinner icon + label) renders under the name; idle rows stay compact with no
  slot. If no fixture agent is `working`, add/verify one in
  `src/fixtures/scenarios/*.ts` (fixed literal values, no Date.now()).
- Attach the shot path in the READY note; record `conclave task gate <ws>
  restore-working-workline -- pnpm uishot laneboard`.
- The **human does final visual acceptance of the motion** (protocol final gate).

## Boundary
`src/components/Roster.tsx`, `src/styles/app.css`

## Risk ledger
- lucide `LoaderCircle` import may already exist or need re-adding — check.
- Don't regress D6's dot token mapping; only remove the `working` branch on the dot.
- app.css line numbers are approximate — grep `roster-working` to anchor.

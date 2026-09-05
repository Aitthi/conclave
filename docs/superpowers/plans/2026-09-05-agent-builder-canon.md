# Agent Builder (New / Edit agent modal) design canon

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Goal

Produce the canonical visual and interaction specification for the redesigned
agent Builder modal (New agent / Edit agent), so the implementation lane can
build from pixels and a checklist instead of prose.

## Reading order

1. `docs/superpowers/specs/2026-09-05-agent-builder-redesign-design.md` — the
   ruled decisions D1–D12 and the ASCII layout. Every decision there is final;
   file a `conclave task challenge` with evidence if one is impossible to draw.
2. `.shots/builder-default.png` — today's modal at commit d90b779 (baseline).
3. `src/components/Builder.tsx:620-1853` — the current JSX, for exact field
   inventory (Model, Effort, Permission mode, Context window, Token filter,
   Custom args, Custom environment, Skills groups, Position).
4. `design/screens/antigravity-cli.tsx` — existing canon for the Antigravity
   runtime block; reuse its component shapes, do not redraw them differently.
5. `design/screens/agent-drafter.tsx` — the CANON comment block convention at
   the top of a canon file.

## Deliverable

Create `design/screens/agent-builder.tsx` (single file, fixed example data,
`export const meta = { title: "Agent Builder — New and Edit agent" }`) with a
preview switch covering at least these states:

- `new-empty`: New agent, nothing filled. Rail: Identity incomplete, Role &
  Level complete, Runtime complete (Claude Code default model), Skills
  complete. Footer left: `Name required`. Primary button disabled.
- `new-filled`: New agent, name "Nova", role Implementer, level Senior,
  Claude Code, model `claude-sonnet-5`, Effort Auto, Permission Bypass (with
  the existing amber warning line), Context 200K, rtk on, Advanced collapsed.
  Footer: `Ready to create`.
- `edit-position`: Edit agent "Tiësto" opened from the roster, five rail
  items including Position, Advanced auto-expanded with a custom arg, footer
  `Ready to save`, primary `Save changes`.
- `edit-antigravity-missing`: Edit agent with Runtime = Antigravity and the
  `agy` binary missing; Runtime rail dot in danger, footer `Install agy to
  continue` in danger colour, primary disabled.
- `dark`: any of the above in the dark theme.

Each state must show the full modal (880 × ≤90vh) inside the app shell
backdrop, plus a second artboard of the Role & Level section alone with
`Custom…` open (inline role editor).

AMENDMENT (human, 2026-09-05, after task creation): the Runtime picker is
NOT a segmented control. It is a 3-column grid of provider tiles, each tile
= provider logo mark (16px, monochrome `currentColor`, brand colour allowed
as the selected accent) + name, in the style of the Untrivial-ai
agent-orchestrator provider table. Draw the three live tiles (Claude Code,
Codex, Antigravity) and, on a separate artboard, the same grid with five
tiles (adding opencode and Muse Spark) so the layout is proven for the
planned runtimes. Logos arrive from task `provider-logos`
(`design/assets/providers/<cliKind>.svg`); until they land, use a
monochrome placeholder glyph and say so in the CANON block. Spec D5 carries
the ruling.

Document in the CANON comment block: rail item states (complete / incomplete
/ error / active), the scroll-spy rule (item highlighted when its section's
top crosses the upper third of the scroll container), the segmented control
sizes, and the copy strings verbatim.

## Constraints

- No product source edits (`src/` untouched).
- Keep the established visual language (tokens from `design/theme.css`,
  existing card / segmented / toggle shapes). This is a layout restructure,
  not a restyle.
- All copy English.
- Fixed literal example data only.
- The canon must render in the design host on `main` — commit it on main via
  `conclave stage commit`, not in a lane worktree.

## Gate

`conclave task gate <ws> agent-builder-canon -- pnpm build` (tsc must stay
green with the new canon file).

## Done

READY note carrying: canon path, pinned commit SHA, and the exact UI
acceptance checklist (one line per visible rule) for a zero-context
implementer. Escalation target: Detoro (30fa04f4).

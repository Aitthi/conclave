# Agent Builder (New / Edit agent modal) Redesign — Design Spec

**Date:** 2026-09-05 · **Owner:** Detoro (lead, 30fa04f4) · **Status:** design approved by human in chat 2026-09-05 (direction, rail mode, and all four content trims ruled); spec pending human review

## Problem

`src/components/Builder.tsx` (1853 lines) is the only place an agent
definition is created or edited. It renders as a 560px single-column modal
whose body is roughly two viewport heights tall: Identity → Level → Role →
Position (edit only) → Type → CLI kind → CLI config → Skills. The Create/Save
button sits below all of it, so the user cannot tell what is still missing
without scrolling to the bottom, and the two biggest blocks (Role as a 2x2
card grid plus a Custom card, Type as three cards of which two are "SOON")
are chosen once and then only take space. New and Edit look identical apart
from the header copy and the Position section.

The human asked for a redesign of this modal ("Re-design model New/Edit
agent"). Baseline pixels: `.shots/builder-default.png` at commit d90b779.

## Decisions (with rejected alternatives)

| # | Decision | Rejected because |
|---|----------|------------------|
| D1 | The modal becomes **880px wide, two columns**: a 180px section rail on the left, scrollable content on the right, header and footer fixed. Human ruling 2026-09-05. | *Wizard for New + single page for Edit* rejected: two layouts to maintain and New users lose the overview. *Keep 560px and only polish* rejected: still two screens of scroll; the human prefers a layout change over squeezing content (standing preference). |
| D2 | The rail is a **scroll-spy over one scrollable page**, not paged panels. Clicking a rail item smooth-scrolls to that section's anchor; the item whose section is in view is highlighted. Human ruling 2026-09-05. | *Paged panels (macOS Settings style)* rejected: errors in a hidden panel need badges, Edit users must click through every panel to review, and hidden required fields complicate the readiness rule. |
| D3 | **Role** renders as one row of four compact cards (icon + name); the selected role's tagline shows under the row. "No role" and "Custom…" are text buttons at the right of the section heading; Custom opens the existing inline role editor. Human ruling 2026-09-05. | 2x2 grid + Custom card rejected: chosen once, largest block on screen. |
| D4 | **Level** is a segmented control `Unranked · Junior · Mid · Senior · Principal` inside the same "Role & Level" section, replacing the four level cards and the "Clear to Unranked" link. Human ruling 2026-09-05. | Four cards rejected (same reason as D3). |
| D5 | **Type + CLI kind collapse into one "Runtime" picker**: a grid of provider tiles (3 columns), each tile = the provider's logo mark (16px) + name, in the style of the Untrivial-ai/agent-orchestrator provider table (human request 2026-09-05, screenshot in chat). Live tiles today: `Claude Code · Codex · Antigravity`. The tile list is driven by one map keyed by `CliKind` (`src/components/builder/providerLogos.tsx`) so the upcoming `opencode` and `Muse Spark` kinds are one entry each when their backend lands; they are NOT rendered until the `CliKind` union carries them. Muted caption under the grid: "Chat agent and Orchestrator are coming soon." The `agentType` state stays `"cli"`; the chat/orchestrator/custom "SOON" cards and the Custom tab are removed from the UI. Human rulings 2026-09-05 (segmented control amended to logo tiles after the human's follow-up). | Plain segmented control rejected by the human: the picker must carry provider logos and scale to five or more runtimes. Keeping SOON cards rejected: non-interactive and cost a full row. Rendering opencode/Muse tiles as disabled placeholders rejected: same non-interactive-row problem; they appear when selectable. |
| D6 | **Custom args and Custom environment move under an "Advanced" disclosure** at the end of Runtime, collapsed by default and auto-expanded when editing a definition that already has either value. Human ruling 2026-09-05. | Always-visible rejected: rarely used fields at the same visual weight as Model. |
| D7 | **Readiness is computed per section** and shown as a dot on each rail item; the footer left slot shows the first blocking reason (`Name required`, `Install agy to continue`, `Checking agy…`) or `Ready to create` / `Ready to save`. The primary button is disabled while any section blocks. An empty model is NOT a blocker: it means "Auto (authenticated default)" today and stays so. | "2 of 4 sections ready" counter rejected: says how many, not what. |
| D8 | **Position stays a section, edit-mode only**, appearing as the fifth rail item under the same `positionEnabled` condition as today. Its content moves unchanged. | Moving Position out of the Builder rejected: out of scope; the Roster chip path already exists. |
| D9 | **State, validation and the save path do not change.** No IPC change, no Rust change, no new command. The redesign is a presentation split: `Builder.tsx` becomes a shell that owns state and renders one component per section from `src/components/builder/`. | Rewriting state management rejected: the risk is in the launch-config edge cases (legacy NULL permission mode, Antigravity availability, role transition skill sync) which are already correct. |
| D10 | **Design canon first.** Arta (designer) produces `design/screens/agent-builder.tsx` on main covering New (empty), New (filled, Claude Code), Edit with Position, Antigravity runtime, and dark theme, before any implementation task is claimable. The Antigravity runtime block follows the existing canon `design/screens/antigravity-cli.tsx`. | Implementing from this spec's text rejected: a UI lane without a canon is a license to improvise (Leadership rule). |
| D11 | **Fixture view `builder-edit`** is added so `pnpm uishot builder-edit` renders the Edit mode (initialDef = first fixture agent def, with a workspace agent id that resolves in `instance.list`) without any browser interaction. | Driving Edit mode through `conclave browser click` rejected: the click verb is unreliable until task `browser-click-reliability` lands, and the blackboard protocol forbids other browsers. |
| D12 | All UI copy English (CONTEXT.md convention). Light and dark themes both required. | — |

## Layout

```
┌─────────────────────────────────────────────────────────────────────┐ 880 × ≤90vh
│ ✦ New agent  [saved to Library]                                  ✕ │ header h-11 (unchanged)
├──────────────┬──────────────────────────────────────────────────────┤
│ ● Identity   │ IDENTITY                     (Drafted by …)          │ rail 180px
│ ● Role&Level │ [avatar] Agent name ______________________          │ content px-6, scrolls
│ ○ Runtime    │ ──────────────────────────────────────────────────   │
│ ● Skills     │ ROLE & LEVEL                    No role · Custom…    │
│   Position*  │ [◎ Lead] [🛡 Reviewer] [🔧 Implementer] [✎ Designer] │ 4 cards, one row
│              │ Settles & delegates work                             │ tagline of selected
│              │ Level  [Unranked|Junior|Mid|Senior|Principal]        │
│              │ ──────────────────────────────────────────────────   │
│              │ RUNTIME                                              │
│              │ [◆ Claude Code] [◉ Codex] [▲ Antigravity]            │ logo tiles, 3 cols
│              │ Chat agent and Orchestrator are coming soon.         │
│              │ Model / Effort / Permission / Context / rtk (as now) │
│              │ ▸ Advanced (custom args, custom environment)         │
│              │ ──────────────────────────────────────────────────   │
│              │ SKILLS (unchanged)                                   │
│              │ ──────────────────────────────────────────────────   │
│              │ POSITION* (unchanged, edit only)                     │
├──────────────┴──────────────────────────────────────────────────────┤
│ Name required                              Cancel   [✦ Create agent]│ footer (fixed)
└─────────────────────────────────────────────────────────────────────┘
* only when positionEnabled (edit mode opened from a roster selection)
```

Rail item states: `complete` (accent dot), `incomplete` (hollow dot),
`error` (danger dot), `active` (bold label + accent left bar, driven by
scroll-spy). Position has no dot: it is always valid.

## Components

All new files under `src/components/builder/`; `Builder.tsx` keeps every
`useState`, effect, derived flag and `handleSave` exactly as today and passes
values + setters down.

| File | Responsibility | Props (shape, not exhaustive) |
|---|---|---|
| `BuilderRail.tsx` | Renders rail items, readiness dots, active highlight; calls `onJump(sectionId)` | `items: {id, label, readiness}[]`, `activeId`, `onJump` |
| `useScrollSpy.ts` | `IntersectionObserver` over the section anchors inside the scroll container; returns `activeId`; exposes `jumpTo(id)` (`scrollIntoView({behavior:"smooth", block:"start"})`) | `containerRef`, `sectionIds` |
| `readiness.ts` | Pure function `sectionReadiness(input) → Record<SectionId, "complete"\|"incomplete"\|"error">` and `firstBlocker(input) → string \| null`. Input is a plain object of the relevant state (`name`, `model`, `cliKind`, `cliAvailability`, `isEditing`). | — |
| `IdentitySection.tsx` | Avatar + colour popover + name + "Drafted by" chip (moved verbatim) | existing identity state |
| `RoleLevelSection.tsx` | D3 + D4; wraps the existing inline custom-role editor | role state, level state, `allSkills`, `applyRoleTransition` |
| `RuntimeSection.tsx` | D5 + D6; owns the segmented runtime control and the Advanced disclosure; the per-CLI config rows (Model, Effort, Permission mode, Context window, rtk, Antigravity availability) move here verbatim | all CLI launch state |
| `SkillsSection.tsx` | Moved verbatim | `allSkills`, `skillIds`, `setSkillIds` |
| `PositionSection.tsx` | Moved verbatim | position state |
| `Section.tsx` | Shared wrapper: `id` anchor (`data-builder-section`), uppercase heading, optional right-slot actions | `id`, `title`, `actions?`, `children` |

Section ids (also the rail order): `identity`, `role`, `runtime`, `skills`,
`position`.

### Readiness rules (D7)

| Section | complete when | error when |
|---|---|---|
| identity | `name.trim().length > 0` | never |
| role | always | never (No role and Unranked are valid) |
| runtime | not Antigravity, OR Antigravity with `cliAvailability.state === "available"` | Antigravity with `cliAvailability.state === "missing"` or `"error"` (`error` = the check itself failed; it shows danger but does NOT block save, matching today's `antigravitySaveBlocked`) |
| skills | always | never |
| position | always | never |

`cliAvailability.state` values are the existing union: `idle`, `checking`,
`available`, `missing`, `error` (`Builder.tsx:52-55`). Antigravity with
`idle` or `checking` is `incomplete`.

`firstBlocker` returns, in order: `"Name required"`, then for Antigravity
`"Install agy to continue"` (missing) or `"Checking agy…"` (idle/checking);
else `null`. The footer shows the blocker in `text-text-tertiary` (danger
colour for the agy-missing case) or `Ready to create` / `Ready to save`. The
primary button is disabled when a blocker exists or `saving` is true; its
label logic is unchanged. Today the button is only disabled for
saving/Antigravity and an empty name fails inside `handleSave`; after this
change the empty-name case is blocked before the click. `handleSave`'s own
name check and `antigravitySaveBlocked` guard stay.

### Runtime picker (D5)

The tile grid drives `cliKind` directly (`claude-code`, `codex`,
`antigravity`). Each tile renders `PROVIDER_LOGOS[cliKind]` (an inline SVG
component, monochrome `currentColor` so it follows the theme, plus the brand
colour as an optional accent when selected) and the provider name. Logos are
sourced by research task `provider-logos` into `design/assets/providers/`
(license recorded there); the implementer inlines them into
`src/components/builder/providerLogos.tsx`. `agentType` is always `"cli"`;
the `showCliConfig` flag stays so the non-CLI Model/API fallback branch still
compiles, but no control can reach it. Remove the `soon` card data and the
Custom tab from the JSX.

Provider names and kinds (today): `claude-code` "Claude Code", `codex`
"Codex", `antigravity` "Antigravity". Planned (human, 2026-09-05; not in
this lane): `opencode` "opencode" (https://opencode.ai), `muse-spark`
"Muse Spark". The logo map must carry entries for all five so the later
backend lane only extends the `CliKind` union.

### Advanced disclosure (D6)

`<details>`-style disclosure built with a button + `aria-expanded`; initial
open state = `Boolean(initialDef?.customArgs) || useCustomEnv`. Contents are
the existing Custom args input (all CLI kinds) and the Custom environment
toggle + textarea (Claude Code only, as today: `isClaudeCode` gate at
`Builder.tsx:1685`), moved verbatim. For Codex and Antigravity the
disclosure contains only Custom args.

## Fixture and screenshot support (D11)

- `src/components/AppShell.tsx`: in the `#view=` map add `"builder-edit"`,
  which sets `builderInitialDef` to the first entry of the fixture
  `agentDef.list` result and `selectedId` to the matching fixture workspace
  agent id so `positionEnabled` is true, then opens the Builder. The fixture
  data already provides `agentDefs[0]` and `instance.list`.
- `scripts/uishot.mjs` usage string and `CLAUDE.md` view-id list gain
  `builder-edit`.
- Required shots before READY (UI pixel gate): `builder` default, `builder`
  empty, `builder-edit` default; each opened and inspected.

## Error handling

Unchanged: `error` state from `handleSave` renders under the last section
in the content column; role save errors render inside the inline editor.
Antigravity availability states render inside Runtime exactly as today.

## Testing and gates

There is no frontend unit-test runner in this repo (`package.json` has no
test script), so verification is:

1. `pnpm build` (tsc + vite) green.
2. `pnpm uishot builder`, `pnpm uishot builder --scenario empty`,
   `pnpm uishot builder-edit`; PNGs opened and inspected; paths in the READY
   note; each run recorded with `conclave task gate`.
3. Manual checklist for the human after rebuild + relaunch: rail click
   scrolls; scrolling moves the highlight; readiness dots and footer blocker
   update as the name is typed; Advanced auto-opens on an edited definition
   with custom args; Antigravity missing state blocks save; dark theme.

## Out of scope

- Any change to `agentDef.save`, role or skill commands, or Rust.
- Chat agent / Orchestrator types (still "coming soon").
- The AgentDrafter overlay and the Library list (they open the Builder
  unchanged through `initialDef`).
- Adding a frontend unit-test runner.

## Work breakdown (lead → tasks)

1. `provider-logos` — Guetta (researcher): source the five provider marks
   (Claude Code, Codex, Antigravity, opencode, Muse Spark) as SVG into
   `design/assets/providers/<cliKind>.svg` with a license note; primary
   source https://github.com/Untrivial-ai/agent-orchestrator. Runs in
   parallel with task 2.
2. `agent-builder-canon` — Arta: `design/screens/agent-builder.tsx` per D10,
   pinned SHA reported in the READY note. Uses the logos from task 1 when
   they land; a monochrome placeholder glyph until then. Escalation: Detoro.
3. `agent-builder-redesign` — Dew: implementation per this spec and the pinned
   canon; boundary `src/components/Builder.tsx`, `src/components/builder/**`,
   `src/components/AppShell.tsx`, `scripts/uishot.mjs`, `CLAUDE.md`.
   Created only after tasks 1 and 2 are READY. Review: Mellow. Escalation:
   Detoro.

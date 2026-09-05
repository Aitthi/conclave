# Agent Builder (New / Edit agent modal) Redesign Implementation Plan

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

> **For agentic workers:** This plan is executed as ONE Conclave lane (`conclave lane start <ws> agent-builder-redesign`). Tasks are sequential and each ends with a commit. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the 560px single-column agent Builder modal into an 880px two-column modal with a scroll-spy section rail, per-section readiness, and the four approved content trims, without changing state, validation, IPC or the save payload.

**Architecture:** `Builder.tsx` keeps every `useState`, effect, derived flag, `handleSave` and helper exactly as today and becomes a shell: header, `BuilderRail`, a scroll container holding five `Section`s, and a footer that shows `firstBlocker`. Each section's JSX is MOVED (not rewritten) into `src/components/builder/<Name>Section.tsx` and receives state + setters as props. New logic is confined to three small pure/hook files: `readiness.ts`, `useScrollSpy.ts`, `BuilderRail.tsx`.

**Tech Stack:** React 19, TypeScript, Tailwind utility classes (existing tokens: `bg-surface`, `ring-overlay/[0.08]`, `text-accent`, `text-danger`, `text-text-tertiary`), lucide-react icons, `pnpm build` (tsc + vite), `pnpm uishot` pixel gate. No test runner exists in this repo; do not add one.

**Spec:** `docs/superpowers/specs/2026-09-05-agent-builder-redesign-design.md` (decisions D1–D12 are final; challenge with evidence, never improvise).

**Design canon:** `design/screens/agent-builder.tsx` pinned at main **b5d3d20** (task `agent-builder-canon`, Arta). Read its CANON comment block and the 35-line acceptance checklist (READY note on that task, also blackboard key `design:agent-builder`) before Task 1. Shots to compare against: `.shots/design-agent-builder-{new-empty,new-filled,edit-advanced,edit-position,edit-antigravity,dark,artboards-bc}.png`. Designer escalation target: Arta (ff647599). Lead: Detoro (30fa04f4).

**Canon token mapping (canon is authored against design-host tokens that `src/styles/app.css` does NOT define — map by role, never copy the name):**

| canon class | app class |
|---|---|
| `bg-fill` | `bg-fill-soft` |
| `border-border` / `divide-border` | `border-overlay/[0.06]` / `divide-overlay/[0.06]` |
| `text-waiting` (amber bypass line) | `text-warning` |
| `bg-live` (toggle on) | `bg-success` |
| `bg-canvas` | `bg-bg-canvas` |
| `text-danger`, `bg-surface`, `text-text-*`, `accent`, `ring-overlay/*` | same |

Canon rulings already folded into this plan: rail active = fill tint (no side bar); scroll-spy clamps `scrollTop <= 4` → first section and at-bottom → last; Position renders its kickers directly on the surface with no outer panel (content unchanged); sections after the first carry `mt-5 border-t border-overlay/[0.06] pt-5`.

## Global Constraints

- Provider marks come ONLY from task `provider-logos` (`design/assets/providers/`); never hand-draw a brand mark.
- All UI copy English. Exact strings from the spec: `Name required`, `Install agy to continue`, `Checking agy…`, `Ready to create`, `Ready to save`, `Chat agent and Orchestrator are coming soon.`, `Advanced`, `Role & Level`, `Runtime`.
- Modal outer: `w-[880px] max-h-[90vh]`. Rail: `w-[180px]`. Content column: `px-6 py-4`.
- No change to any `ipc.*` call, to `handleSave`'s payload, or to any file outside the boundary: `src/components/Builder.tsx`, `src/components/builder/**`, `src/components/AppShell.tsx`, `scripts/uishot.mjs`, `CLAUDE.md`.
- Never call `Date.now()` or add fixture handlers that use it (fixture rule in `CLAUDE.md`).
- Move JSX verbatim: when a task says "move lines A–B", cut those lines, paste them into the new file, and change ONLY the identifiers that become props. Do not restyle rows the spec does not name.
- Before every commit: `pnpm build` green. Commit with `conclave stage commit` (never bare `git commit` in a shared checkout); in a lane worktree plain `git commit` with an explicit pathspec is acceptable.
- Before READY: `pnpm uishot builder`, `pnpm uishot builder --scenario empty`, `pnpm uishot builder-edit`, each PNG opened and inspected, each run recorded with `conclave task gate <ws> agent-builder-redesign -- pnpm uishot <view>`. Check `lsof -nP -iTCP:1420 -sTCP:LISTEN` first and kill any dev server that belongs to another checkout.
- Fresh lane worktrees have no `node_modules`: run `pnpm install` once first.
- Do not edit `src/lib/positions.ts`, `src/lib/modelCatalogue.ts`, or `src/components/Position.tsx`; import from them.

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/components/builder/readiness.ts` | create | Pure: `sectionReadiness(input)` and `firstBlocker(input)` |
| `src/components/builder/useScrollSpy.ts` | create | Hook: `activeId` from an `IntersectionObserver`, `jumpTo(id)` |
| `src/components/builder/Section.tsx` | create | Anchor wrapper with uppercase heading + optional right-slot actions |
| `src/components/builder/BuilderRail.tsx` | create | Rail list with readiness dots and active highlight |
| `src/components/builder/IdentitySection.tsx` | create | Moved from `Builder.tsx` Identity section |
| `src/components/builder/RoleLevelSection.tsx` | create | Role row (D3) + Level segmented control (D4) + moved callout/editor |
| `src/components/builder/providerLogos.tsx` | create | Provider logo marks + `RUNTIME_TILES` (D5) |
| `src/components/builder/RuntimeSection.tsx` | create | Runtime logo tiles (D5) + moved CLI config rows + Advanced (D6) |
| `src/components/builder/SkillsSection.tsx` | create | Moved Skills section |
| `src/components/builder/PositionSection.tsx` | create | Moved Position section |
| `src/components/Builder.tsx` | modify | Shell: state (unchanged) + layout + footer blocker |
| `src/components/AppShell.tsx` | modify | `builder-edit` fixture view (D11) |
| `scripts/uishot.mjs` | modify | usage string lists `builder-edit` |
| `CLAUDE.md` | modify | view-id list gains `builder-edit` |

Section ids and rail order: `identity`, `role`, `runtime`, `skills`, `position`.

---

### Task 1: Readiness model (pure)

**Files:**
- Create: `src/components/builder/readiness.ts`

**Interfaces:**
- Produces:
  ```ts
  export type SectionId = "identity" | "role" | "runtime" | "skills" | "position";
  export type Readiness = "complete" | "incomplete" | "error";
  export type CliAvailabilityState = "idle" | "checking" | "available" | "missing" | "error";
  export interface ReadinessInput {
    name: string;
    isAntigravity: boolean;
    cliAvailabilityState: CliAvailabilityState;
    isEditing: boolean;
  }
  export function sectionReadiness(input: ReadinessInput): Record<SectionId, Readiness>;
  export function firstBlocker(input: ReadinessInput): string | null;
  export function readyLabel(input: ReadinessInput): string; // "Ready to create" | "Ready to save"
  ```

- [ ] **Step 1: Write the file**

```ts
// src/components/builder/readiness.ts
//
// Per-section readiness for the agent Builder (spec D7). Pure: the shell
// passes a plain snapshot of its state, the rail and footer render the result.
// An empty model is NOT a blocker — it means "Auto (authenticated default)".

export type SectionId = "identity" | "role" | "runtime" | "skills" | "position";
export type Readiness = "complete" | "incomplete" | "error";
export type CliAvailabilityState = "idle" | "checking" | "available" | "missing" | "error";

export interface ReadinessInput {
  name: string;
  isAntigravity: boolean;
  cliAvailabilityState: CliAvailabilityState;
  isEditing: boolean;
}

export const SECTION_ORDER: SectionId[] = ["identity", "role", "runtime", "skills", "position"];

export const SECTION_LABELS: Record<SectionId, string> = {
  identity: "Identity",
  role: "Role & Level",
  runtime: "Runtime",
  skills: "Skills",
  position: "Position",
};

function runtimeReadiness(input: ReadinessInput): Readiness {
  if (!input.isAntigravity) return "complete";
  switch (input.cliAvailabilityState) {
    case "available":
      return "complete";
    case "missing":
    case "error":
      return "error";
    default:
      return "incomplete";
  }
}

export function sectionReadiness(input: ReadinessInput): Record<SectionId, Readiness> {
  return {
    identity: input.name.trim().length > 0 ? "complete" : "incomplete",
    role: "complete",
    runtime: runtimeReadiness(input),
    skills: "complete",
    position: "complete",
  };
}

/** First reason the primary button is disabled, in display order; null = ready. */
export function firstBlocker(input: ReadinessInput): string | null {
  if (input.name.trim().length === 0) return "Name required";
  if (input.isAntigravity) {
    if (input.cliAvailabilityState === "missing") return "Install agy to continue";
    if (input.cliAvailabilityState === "idle" || input.cliAvailabilityState === "checking") {
      return "Checking agy…";
    }
  }
  return null;
}

export function readyLabel(input: ReadinessInput): string {
  return input.isEditing ? "Ready to save" : "Ready to create";
}

/** True when the blocker should render in the danger colour. */
export function blockerIsDanger(blocker: string | null): boolean {
  return blocker === "Install agy to continue";
}
```

- [ ] **Step 2: Type-check**

Run: `pnpm build`
Expected: exits 0 (file is unused yet; tsc still compiles it).

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(builder): per-section readiness model (spec D7)" -- src/components/builder/readiness.ts
```

---

### Task 2: Scroll-spy hook and Section wrapper

**Files:**
- Create: `src/components/builder/useScrollSpy.ts`
- Create: `src/components/builder/Section.tsx`

**Interfaces:**
- Produces:
  ```ts
  export function useScrollSpy(containerRef: React.RefObject<HTMLElement | null>, ids: string[]): { activeId: string; jumpTo: (id: string) => void };
  export function Section(props: { id: SectionId; title: string; actions?: React.ReactNode; children: React.ReactNode }): JSX.Element;
  export const SECTION_ATTR = "data-builder-section";
  ```

- [ ] **Step 1: Write the hook**

```ts
// src/components/builder/useScrollSpy.ts
//
// Highlights the rail item whose section is in view (spec D2). A section is
// "active" when its top edge is inside the upper third of the scroll
// container; the LAST such section wins so scrolling down advances the
// highlight in order. jumpTo() smooth-scrolls a section to the top.

import { useCallback, useEffect, useState, type RefObject } from "react";

export const SECTION_ATTR = "data-builder-section";

export function useScrollSpy(
  containerRef: RefObject<HTMLElement | null>,
  ids: string[],
): { activeId: string; jumpTo: (id: string) => void } {
  const [activeId, setActiveId] = useState<string>(ids[0] ?? "");

  useEffect(() => {
    const root = containerRef.current;
    if (!root) return;
    const sections = ids
      .map((id) => root.querySelector<HTMLElement>(`[${SECTION_ATTR}="${id}"]`))
      .filter((el): el is HTMLElement => el !== null);
    if (sections.length === 0) return;

    const recompute = () => {
      const rootTop = root.getBoundingClientRect().top;
      const threshold = root.clientHeight / 3;
      let current = sections[0].getAttribute(SECTION_ATTR) ?? "";
      // Canon clamp: at rest (scrollTop <= 4) the first section is active.
      if (root.scrollTop <= 4) {
        setActiveId(current);
        return;
      }
      for (const el of sections) {
        const top = el.getBoundingClientRect().top - rootTop;
        if (top <= threshold) current = el.getAttribute(SECTION_ATTR) ?? current;
      }
      // Bottom of the scroll range: the last section may never reach the
      // upper third, so treat "scrolled to the end" as "last section active".
      if (root.scrollTop + root.clientHeight >= root.scrollHeight - 1) {
        current = sections[sections.length - 1].getAttribute(SECTION_ATTR) ?? current;
      }
      setActiveId(current);
    };

    recompute();
    const observer = new IntersectionObserver(recompute, {
      root,
      threshold: [0, 0.25, 0.5, 0.75, 1],
    });
    sections.forEach((el) => observer.observe(el));
    root.addEventListener("scroll", recompute, { passive: true });
    return () => {
      observer.disconnect();
      root.removeEventListener("scroll", recompute);
    };
    // ids is a stable module-level list in the shell; join() keeps the dep primitive.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [containerRef, ids.join("|")]);

  const jumpTo = useCallback(
    (id: string) => {
      const root = containerRef.current;
      const el = root?.querySelector<HTMLElement>(`[${SECTION_ATTR}="${id}"]`);
      if (!el) return;
      el.scrollIntoView({ behavior: "smooth", block: "start" });
      setActiveId(id);
    },
    [containerRef],
  );

  return { activeId, jumpTo };
}
```

- [ ] **Step 2: Write the Section wrapper**

```tsx
// src/components/builder/Section.tsx
//
// One Builder section: the scroll-spy anchor, the uppercase heading used by
// every section today (Builder.tsx heading classes), and an optional
// right-slot for heading actions (e.g. "No role · Custom…").

import type { ReactNode } from "react";
import type { SectionId } from "./readiness";
import { SECTION_ATTR } from "./useScrollSpy";

interface SectionProps {
  id: SectionId;
  title: string;
  actions?: ReactNode;
  children: ReactNode;
}

export function Section({ id, title, actions, children }: SectionProps) {
  return (
    <section
      {...{ [SECTION_ATTR]: id }}
      id={`builder-${id}`}
      aria-labelledby={`builder-${id}-heading`}
      className="scroll-mt-4"
    >
      <div className="flex items-center justify-between gap-2 mb-2">
        <div
          id={`builder-${id}-heading`}
          className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase"
        >
          {title}
        </div>
        {actions}
      </div>
      {children}
    </section>
  );
}
```

- [ ] **Step 3: Type-check**

Run: `pnpm build`
Expected: exits 0.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(builder): scroll-spy hook and Section anchor wrapper (spec D2)" -- src/components/builder/useScrollSpy.ts src/components/builder/Section.tsx
```

---

### Task 3: BuilderRail

**Files:**
- Create: `src/components/builder/BuilderRail.tsx`

**Interfaces:**
- Consumes: `SectionId`, `Readiness`, `SECTION_LABELS` from Task 1.
- Produces:
  ```ts
  export function BuilderRail(props: { items: SectionId[]; readiness: Record<SectionId, Readiness>; activeId: string; onJump: (id: SectionId) => void }): JSX.Element;
  ```

- [ ] **Step 1: Write the rail**

```tsx
// src/components/builder/BuilderRail.tsx
//
// Left rail of the Builder (spec D1/D7). One row per section: readiness dot,
// label, accent fill tint when active. Position (edit only) never shows a dot.
// Sizes and colours per canon design/screens/agent-builder.tsx.

import type { Readiness, SectionId } from "./readiness";
import { SECTION_LABELS } from "./readiness";

interface BuilderRailProps {
  items: SectionId[];
  readiness: Record<SectionId, Readiness>;
  activeId: string;
  onJump: (id: SectionId) => void;
}

function dotClass(state: Readiness): string {
  switch (state) {
    case "complete":
      return "bg-accent";
    case "error":
      return "bg-danger";
    default:
      return "ring-1 ring-inset ring-overlay/[0.25] bg-transparent";
  }
}

export function BuilderRail({ items, readiness, activeId, onJump }: BuilderRailProps) {
  return (
    <nav aria-label="Builder sections" className="w-[180px] shrink-0 border-r border-overlay/[0.06] py-3 pr-2 pl-3">
      <ul className="space-y-0.5">
        {items.map((id) => {
          const active = id === activeId;
          const state = readiness[id];
          return (
            <li key={id}>
              <button
                type="button"
                onClick={() => onJump(id)}
                aria-current={active ? "location" : undefined}
                data-readiness={id === "position" ? undefined : state}
                className={`flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-[12.5px] transition-colors ${
                  active
                    ? "font-semibold text-accent bg-accent/[0.10]"
                    : "text-text-secondary hover:bg-overlay/[0.03]"
                }`}
              >
                {/* Active = fill tint only. No border-left bar: that is the
                    side-tab antipattern (Arta challenge 69718db4, slop-detect). */}
                {id !== "position" && (
                  <span
                    aria-hidden
                    className={`h-2 w-2 shrink-0 rounded-full ${dotClass(state)}`}
                  />
                )}
                <span className="truncate">{SECTION_LABELS[id]}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
```

- [ ] **Step 2: Type-check**

Run: `pnpm build`
Expected: exits 0. If `text-text-primary` is not a token in `src/index.css` / tailwind config, grep for the primary text token used by `Roster.tsx` headings and use that instead (do not invent a token).

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(builder): section rail with readiness dots" -- src/components/builder/BuilderRail.tsx
```

---

### Task 4: Move Identity, Skills and Position sections verbatim

This task only relocates JSX so later tasks work on a small shell. No visual change is expected after this task; take `pnpm uishot builder` before and after and compare by eye.

**Files:**
- Create: `src/components/builder/IdentitySection.tsx`
- Create: `src/components/builder/SkillsSection.tsx`
- Create: `src/components/builder/PositionSection.tsx`
- Modify: `src/components/Builder.tsx` (Identity `<section>` at ~652–745, Position `<section>` at ~991–1150, Skills `<section>` at ~1725–1800; line numbers are at commit d90b779 — re-locate by the section comments `{/* Identity */}`, `{positionEnabled && (`, `{/* Skills — for every first-class CLI harness. */}`)

**Interfaces:**
- Produces:
  ```ts
  export function IdentitySection(props: {
    name: string; setName: (v: string) => void;
    color: string; setColor: (v: string) => void;
    showColors: boolean; setShowColors: (v: boolean) => void;
    letter: string; draftedBy?: string; touched: boolean; setTouched: (v: boolean) => void;
  }): JSX.Element;
  export function SkillsSection(props: { allSkills: Skill[]; skillIds: string[]; setSkillIds: React.Dispatch<React.SetStateAction<string[]>> }): JSX.Element;
  export function PositionSection(props: {
    scopedAgent: WorkspaceAgent; positionRoster: WorkspaceAgent[]; supervisorOptions: WorkspaceAgent[];
    previewRoster: WorkspaceAgent[]; previewChainIds: string[]; trackLabel: string;
    levelDraft: string | null; setLevelDraft: (v: string | null) => void;
    supervisorDraft: string | null; setSupervisorDraft: (v: string | null) => void;
  }): JSX.Element;
  ```
  If a moved block references a state, helper, or import not in this list, ADD it to the props (and to this Interfaces block in the plan) rather than duplicating the state.

- [ ] **Step 1: Create `IdentitySection.tsx`**

Cut the Identity `<section>…</section>` from `Builder.tsx` and paste it as the return value of `IdentitySection`, replacing the outer `<section>` + heading `<div>` with `<Section id="identity" title="Identity" actions={draftedBy && !touched ? (<span …>Drafted by …</span>) : undefined}>` — the "Drafted by" chip becomes the heading's right-slot `actions`, its markup unchanged. Import `COLOR_SWATCHES` from `../../lib/modelCatalogue` and the lucide icons it uses (`Sparkles`, `Check`, and whatever the colour popover uses — copy the exact imports from `Builder.tsx`).

- [ ] **Step 2: Create `SkillsSection.tsx`**

Cut the Skills `<section>` and wrap with `<Section id="skills" title="Skills">`. The inner `rounded-xl ring-1 …` box and its three groups move verbatim.

- [ ] **Step 3: Create `PositionSection.tsx`**

Cut the Position `<section>` body (inside `{positionEnabled && (…)}`) and wrap with `<Section id="position" title="Position">`. Keep the `HumanChip`, `PositionLine`, `levelOf`, `wouldCycle`, `LEVELS` imports it needs (`../Position`, `../../lib/positions`). Canon rule 29: drop the outer `rounded-xl ring-1 … p-3` panel that wraps Track / Level / Supervisor / Escalation chain today; the four kickers and their content sit directly on the surface (`space-y-3`), otherwise unchanged. Rule 30: Position KEEPS its four Level cards (it edits the live instance), distinct from Role & Level's segmented control (definition default).

- [ ] **Step 4: Render the three components from `Builder.tsx`**

Replace each removed block with the component call passing the props listed above, e.g.

```tsx
<IdentitySection
  name={name} setName={(v) => { setName(v); setTouched(true); }}
  color={color} setColor={setColor}
  showColors={showColors} setShowColors={setShowColors}
  letter={letter} draftedBy={draftedBy} touched={touched} setTouched={setTouched}
/>
```

Only wrap `setName` with `setTouched(true)` if the original `onChange` already did so — check the cut code and preserve its exact behaviour.

- [ ] **Step 5: Type-check and screenshot**

Run: `pnpm build` → exit 0.
Run: `pnpm uishot builder` and open `.shots/builder-default.png` → identical layout to before this task apart from nothing.

- [ ] **Step 6: Commit**

```bash
git commit -m "refactor(builder): move Identity, Skills, Position sections into builder/" -- src/components/Builder.tsx src/components/builder/IdentitySection.tsx src/components/builder/SkillsSection.tsx src/components/builder/PositionSection.tsx
```

---

### Task 5: Role & Level section (D3 + D4)

**Files:**
- Create: `src/components/builder/RoleLevelSection.tsx`
- Modify: `src/components/Builder.tsx` (remove the Level `<section>` ~746–788 and the Role `<section>` ~789–989)

**Interfaces:**
- Consumes: `roleLook(role)` — MOVE this function and `BUILTIN_ROLE_LOOKS` from `Builder.tsx` into `RoleLevelSection.tsx` and export `roleLook` from there (Builder does not use it elsewhere; verify with `conclave code refs roleLook`).
- Produces:
  ```ts
  export function RoleLevelSection(props: {
    orderedRoles: Role[]; selectedRole: Role | undefined; roleId: string;
    selectRole: (id: string) => void;
    clearRole: () => void;            // the existing "No role" onClick body
    openCustomRole: () => void;       // the existing "Custom…" onClick body
    customRoleOpen: boolean;
    customRoleName: string; setCustomRoleName: (v: string) => void;
    customRoleDesc: string; setCustomRoleDesc: (v: string) => void;
    customRoleSkillIds: string[]; setCustomRoleSkillIds: React.Dispatch<React.SetStateAction<string[]>>;
    cancelCustomRole: () => void;     // the existing editor Cancel onClick body
    handleCreateCustomRole: () => void; savingRole: boolean;
    allSkills: Skill[]; attachSkillNames: string[]; mandatorySkillNames: string[];
    defaultLevel: AgentDefinition["defaultLevel"]; setDefaultLevel: (v: AgentDefinition["defaultLevel"]) => void;
  }): JSX.Element;
  ```

- [ ] **Step 1: Heading actions**

```tsx
<Section
  id="role"
  title="Role & Level"
  actions={
    <div className="flex items-center gap-3">
      <button type="button" onClick={clearRole}
        className={`text-[11px] font-medium transition-colors ${roleId === "" && !customRoleOpen ? "text-accent" : "text-text-tertiary hover:text-text-secondary"}`}>
        No role
      </button>
      <button type="button" onClick={openCustomRole} aria-pressed={customRoleOpen}
        className={`inline-flex items-center gap-1 text-[11px] font-medium transition-colors ${customRoleOpen ? "text-accent" : "text-text-tertiary hover:text-text-secondary"}`}>
        <Plus className="w-3 h-3" /> Custom…
      </button>
    </div>
  }
>
```

- [ ] **Step 2: Role row (one line of compact cards)**

Replace the `grid grid-cols-2 gap-2` grid and the dashed Custom card with:

```tsx
<div role="radiogroup" aria-label="Role" className="grid grid-cols-4 gap-2">
  {orderedRoles.map((r) => {
    const { Icon } = roleLook(r);
    const active = roleId === r.id && !customRoleOpen;
    return (
      <button key={r.id} type="button" role="radio" aria-checked={active} onClick={() => selectRole(r.id)}
        className={`flex items-center gap-2 rounded-xl px-2.5 py-2 text-left ring-1 transition-all ${
          active ? "ring-accent/40 bg-accent/[0.06]" : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"}`}>
        <Icon className={`w-4 h-4 shrink-0 ${active ? "text-accent" : "text-text-secondary"}`} />
        <span className="text-[12.5px] font-semibold leading-tight truncate">{r.name}</span>
      </button>
    );
  })}
</div>
```

Custom roles beyond the four builtins wrap to a second row of the same grid; that is acceptable (canon shows four).

- [ ] **Step 3: Tagline under the row, then the moved callout / no-role note / inline editor**

Directly under the grid:

```tsx
<p className="mt-1.5 text-[11px] text-text-tertiary leading-snug min-h-[16px]">
  {selectedRole ? roleLook(selectedRole).tagline : customRoleOpen ? "Define your own role" : "No role — mandatory skills only"}
</p>
```

Then paste the existing selected-role callout, the no-role dashed note, and the inline custom-role editor verbatim (they were below the grid before).

- [ ] **Step 4: Level segmented control**

Under the role blocks, replace the four level cards with:

```tsx
<div className="mt-3 flex items-center justify-between gap-3">
  <span className="text-[12.5px] text-text-secondary">Level</span>
  <div role="radiogroup" aria-label="Level" className="grid grid-cols-5 gap-1 rounded-xl bg-overlay/[0.04] p-1 w-[420px]">
    {[{ id: null, name: "Unranked" }, ...LEVELS.map((l) => ({ id: l.id, name: l.name }))].map((opt) => {
      const active = (defaultLevel ?? null) === opt.id;
      return (
        <button key={opt.name} type="button" role="radio" aria-checked={active}
          onClick={() => setDefaultLevel(opt.id as AgentDefinition["defaultLevel"])}
          className={`min-w-0 rounded-lg px-1 py-1.5 text-[11.5px] transition-colors ${
            active ? "bg-surface shadow-sm font-semibold" : "text-text-secondary hover:bg-overlay/[0.03]"}`}>
          <span className="block truncate">{opt.name}</span>
        </button>
      );
    })}
  </div>
</div>
```

`LEVELS` comes from `../../lib/positions` (ids `junior|mid|senior|principal`, in rung order — verify in that file, do not hardcode names).

- [ ] **Step 5: Wire from `Builder.tsx`**

Remove the old Level and Role sections and render:

```tsx
<RoleLevelSection
  orderedRoles={orderedRoles} selectedRole={selectedRole} roleId={roleId}
  selectRole={selectRole}
  clearRole={() => { applyRoleTransition(roleId); setRoleId(""); setCustomRoleOpen(false); }}
  openCustomRole={() => { applyRoleTransition(roleId); setCustomRoleOpen(true); setRoleId(""); }}
  customRoleOpen={customRoleOpen}
  customRoleName={customRoleName} setCustomRoleName={setCustomRoleName}
  customRoleDesc={customRoleDesc} setCustomRoleDesc={setCustomRoleDesc}
  customRoleSkillIds={customRoleSkillIds} setCustomRoleSkillIds={setCustomRoleSkillIds}
  cancelCustomRole={() => { setCustomRoleOpen(false); setCustomRoleName(""); setCustomRoleDesc(""); setCustomRoleSkillIds([]); }}
  handleCreateCustomRole={handleCreateCustomRole} savingRole={savingRole}
  allSkills={allSkills} attachSkillNames={attachSkillNames} mandatorySkillNames={mandatorySkillNames}
  defaultLevel={defaultLevelDraft} setDefaultLevel={setDefaultLevelDraft}
/>
```

The three inline closures above are the EXISTING onClick bodies copied from the removed JSX; if the original bodies differ from what is written here, the original wins.

- [ ] **Step 6: Type-check, screenshot, commit**

Run: `pnpm build` → 0. `pnpm uishot builder` → open PNG: role row of four compact cards, tagline, Level segmented control.

```bash
git commit -m "feat(builder): Role & Level section — compact role row, level segmented control (spec D3/D4)" -- src/components/Builder.tsx src/components/builder/RoleLevelSection.tsx
```

---

### Task 6: Runtime section (D5 + D6)

**Files:**
- Create: `src/components/builder/RuntimeSection.tsx`
- Modify: `src/components/Builder.tsx` (remove the Type `<section>` ~1152–1243, the `!showCliConfig` Model fallback ~1244–1267, and the CLI config `<section>` ~1269–1723)

**Interfaces:**
- Produces:
  ```ts
  export function RuntimeSection(props: {
    cliKind: CliKind; selectCliKind: (k: CliKind) => void;
    isClaudeCode: boolean; isCodex: boolean; isAntigravity: boolean;
    cliAvailability: CliAvailability; checkAntigravityAvailability: () => void; openAntigravityInstallGuide: () => void;
    modelCatalog: CliModelCatalog; loadAntigravityModels: () => void; catalogModels: {id: string; label: string}[]; savedModelUnlisted: boolean;
    model: string; setModel: (v: string) => void; modelPresets: readonly string[]; selectModelPreset: (m: string) => void;
    effort: CliEffort; setEffort: (v: CliEffort) => void;
    permissionMode: PermissionMode; setPermissionMode: (v: PermissionMode) => void; setPermissionModeDirty: (v: boolean) => void;
    contextWindow: string; setContextWindow: (v: string) => void;
    rtkEnabled: boolean; setRtkEnabled: (v: boolean) => void;
    customArgs: string; setCustomArgs: (v: string) => void;
    useCustomEnv: boolean; setUseCustomEnv: (v: boolean) => void; envText: string; setEnvText: (v: string) => void;
    advancedInitiallyOpen: boolean;
  }): JSX.Element;
  ```
  The types `CliKind`, `PermissionMode`, `CliEffort`, `CliAvailability`, `CliModelCatalog` and the constants `ANTIGRAVITY_MODE_HELP`, `SECRET_PLACEHOLDER` (check where it is defined; import, don't copy) and the `Toggle` component MOVE from `Builder.tsx` to `RuntimeSection.tsx` and are exported from there; `Builder.tsx` imports them back. `AgentType` stays in `Builder.tsx`.

- [ ] **Step 0: Provider logo map** — create `src/components/builder/providerLogos.tsx`

Inline the SVGs delivered by task `provider-logos` (`design/assets/providers/<cliKind>.svg`, READY note names the commit) as React components. Every path uses `fill="currentColor"` (or `stroke="currentColor"`); no hardcoded fills, so the mark follows the theme.

```tsx
// src/components/builder/providerLogos.tsx
//
// Provider marks for the Runtime picker (spec D5). Keyed by the runtime
// kind string so the upcoming opencode / muse-spark kinds are one entry each:
// the picker renders only the kinds present in RUNTIME_TILES.
import type { SVGProps } from "react";

export type ProviderKind = "claude-code" | "codex" | "antigravity" | "opencode" | "muse-spark";

type Mark = (props: SVGProps<SVGSVGElement>) => JSX.Element;

// One component per file in design/assets/providers/; paths pasted verbatim.
const ClaudeCodeMark: Mark = (p) => (<svg viewBox="0 0 16 16" width={16} height={16} aria-hidden {...p}>{/* paths from claude-code.svg */}</svg>);
const CodexMark: Mark = (p) => (<svg viewBox="0 0 16 16" width={16} height={16} aria-hidden {...p}>{/* codex.svg */}</svg>);
const AntigravityMark: Mark = (p) => (<svg viewBox="0 0 16 16" width={16} height={16} aria-hidden {...p}>{/* antigravity.svg */}</svg>);
const OpencodeMark: Mark = (p) => (<svg viewBox="0 0 16 16" width={16} height={16} aria-hidden {...p}>{/* opencode.svg */}</svg>);
const MuseSparkMark: Mark = (p) => (<svg viewBox="0 0 16 16" width={16} height={16} aria-hidden {...p}>{/* muse-spark.svg */}</svg>);

export const PROVIDER_LOGOS: Record<ProviderKind, { name: string; Mark: Mark }> = {
  "claude-code": { name: "Claude Code", Mark: ClaudeCodeMark },
  codex: { name: "Codex", Mark: CodexMark },
  antigravity: { name: "Antigravity", Mark: AntigravityMark },
  opencode: { name: "opencode", Mark: OpencodeMark },
  "muse-spark": { name: "Muse Spark", Mark: MuseSparkMark },
};

/** Kinds the backend can launch today — the ONLY ones the picker renders. */
export const RUNTIME_TILES: ProviderKind[] = ["claude-code", "codex", "antigravity"];
```

Delivered marks (task `provider-logos`, main commits a65f6b8 + 2060481): `claude-code.svg`, `codex.svg`, `antigravity.svg`, `opencode.svg` — all `viewBox="0 0 24 24"`, `currentColor`; render them at `width={16} height={16}` keeping the 24 viewBox. `muse-spark` has NO vector (only `muse-spark.png`, ruled by Detoro 2026-09-05): its map entry uses a 16px `lucide-react` `Terminal` icon until Meta publishes a vector; do not embed the PNG and never draw a logo yourself.

- [ ] **Step 1: Runtime logo tiles + caption (replaces Type cards + CLI kind row)**

```tsx
<Section id="runtime" title="Runtime" actions={antigravityStatusChip /* the existing agy availability <span> from the CLI-config heading, moved here */}>
  <div role="radiogroup" aria-label="Runtime" className="grid grid-cols-3 gap-2">
    {RUNTIME_TILES.map((kind) => {
      const { name, Mark } = PROVIDER_LOGOS[kind];
      const active = cliKind === kind;
      return (
        <button key={kind} type="button" role="radio" aria-checked={active} onClick={() => selectCliKind(kind as CliKind)}
          className={`flex items-center gap-2 rounded-xl px-2.5 py-2 text-left ring-1 transition-all ${
            active ? "ring-accent/40 bg-accent/[0.06]" : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"}`}>
          <Mark className={`shrink-0 ${active ? "text-accent" : "text-text-secondary"}`} />
          <span className="text-[12.5px] font-semibold leading-tight truncate">{name}</span>
        </button>
      );
    })}
  </div>
  <p className="mt-1.5 mb-3 text-[10.5px] text-text-tertiary">Chat agent and Orchestrator are coming soon.</p>
  {/* Antigravity missing/error alert box — moved verbatim */}
  {/* The rounded-xl config box — moved verbatim: Model, Effort, Permission mode, Context window, Token filter */}
  {/* Advanced disclosure — Step 2 */}
  {/* Antigravity "Token filtering and sandbox controls…" note — moved verbatim */}
</Section>
```

Delete the `soon` entries, the `Custom` CLI tab, and the `!showCliConfig` Model/API fallback block entirely. `agentType` remains `"cli"` in `Builder.tsx`; keep `const [agentType] = useState<AgentType>("cli")` (drop the unused setter to satisfy lint) — `handleSave` still reads it.

- [ ] **Step 2: Advanced disclosure**

Inside `RuntimeSection`, after the config box:

```tsx
const [advancedOpen, setAdvancedOpen] = useState(advancedInitiallyOpen);
…
<div className="mt-2 rounded-xl ring-1 ring-overlay/[0.08] bg-surface">
  <button type="button" onClick={() => setAdvancedOpen((v) => !v)} aria-expanded={advancedOpen} aria-controls="builder-advanced"
    className="flex w-full items-center justify-between px-3 py-2 text-[12.5px] text-text-secondary hover:bg-overlay/[0.02] rounded-xl">
    <span className="inline-flex items-center gap-1.5">
      <ChevronDown className={`w-3.5 h-3.5 transition-transform ${advancedOpen ? "" : "-rotate-90"}`} />
      Advanced
    </span>
    <span className="text-[10.5px] text-text-tertiary">{isClaudeCode ? "Custom args, custom environment" : "Custom args"}</span>
  </button>
  {advancedOpen && (
    <div id="builder-advanced" className="divide-y divide-overlay/[0.06] border-t border-overlay/[0.06]">
      {/* Custom args row — moved verbatim */}
      {/* Custom env block — moved verbatim, still wrapped in {isClaudeCode && (…)} */}
    </div>
  )}
</div>
```

- [ ] **Step 3: Wire from `Builder.tsx`**

```tsx
<RuntimeSection
  cliKind={cliKind} selectCliKind={selectCliKind}
  isClaudeCode={isClaudeCode} isCodex={isCodex} isAntigravity={isAntigravity}
  cliAvailability={cliAvailability} checkAntigravityAvailability={() => void checkAntigravityAvailability()} openAntigravityInstallGuide={() => void openAntigravityInstallGuide()}
  modelCatalog={modelCatalog} loadAntigravityModels={() => void loadAntigravityModels()} catalogModels={catalogModels} savedModelUnlisted={savedModelUnlisted}
  model={model} setModel={setModel} modelPresets={modelPresets} selectModelPreset={selectModelPreset}
  effort={effort} setEffort={setEffort}
  permissionMode={permissionMode} setPermissionMode={setPermissionMode} setPermissionModeDirty={setPermissionModeDirty}
  contextWindow={contextWindow} setContextWindow={setContextWindow}
  rtkEnabled={rtkEnabled} setRtkEnabled={setRtkEnabled}
  customArgs={customArgs} setCustomArgs={setCustomArgs}
  useCustomEnv={useCustomEnv} setUseCustomEnv={setUseCustomEnv} envText={envText} setEnvText={setEnvText}
  advancedInitiallyOpen={Boolean(initialDef?.customArgs) || useCustomEnv}
/>
```

`advancedInitiallyOpen` is read once (initial `useState`), so passing the live `useCustomEnv` is fine.

- [ ] **Step 4: Type-check, screenshots, commit**

Run: `pnpm build` → 0. `pnpm uishot builder` → runtime segmented control, caption, config box, collapsed Advanced. Also verify the Antigravity states still render: temporarily switch nothing — the fixture default is Claude Code; the Antigravity path is covered by the canon and the human checklist.

```bash
git commit -m "feat(builder): Runtime section — provider logo tiles, Advanced disclosure (spec D5/D6)" -- src/components/Builder.tsx src/components/builder/RuntimeSection.tsx src/components/builder/providerLogos.tsx
```

---

### Task 7: Shell layout — rail, scroll container, footer blocker (D1, D2, D7)

**Files:**
- Modify: `src/components/Builder.tsx` render block (the `return (` at ~625 to the end)

- [ ] **Step 1: Readiness + scroll-spy in the shell**

Above `return`:

```tsx
const railItems: SectionId[] = positionEnabled ? SECTION_ORDER : SECTION_ORDER.filter((id) => id !== "position");
const readinessInput: ReadinessInput = {
  name, isAntigravity, cliAvailabilityState: cliAvailability.state, isEditing,
};
const readiness = sectionReadiness(readinessInput);
const blocker = firstBlocker(readinessInput);
const scrollRef = useRef<HTMLDivElement>(null);
const { activeId, jumpTo } = useScrollSpy(scrollRef, railItems);
```

Hooks must stay above any early return; `useRef` joins the existing `react` import.

- [ ] **Step 2: Replace the outer layout**

```tsx
<div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30">
  <div className="w-[880px] max-h-[90vh] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]">
    {/* header — unchanged */}
    <div className="flex flex-1 min-h-0">
      <BuilderRail items={railItems} readiness={readiness} activeId={activeId} onJump={jumpTo} />
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-6 py-4 space-y-5 min-h-0">
        <IdentitySection … />
        <RoleLevelSection … />
        <RuntimeSection … />
        <SkillsSection … />
        {positionEnabled && <PositionSection … />}
        {error && <p className="text-[12px] text-danger px-1">{error}</p>}
      </div>
    </div>
    {/* footer */}
    <div className="border-t border-overlay/[0.07] px-5 py-2.5 bg-surface shrink-0 flex items-center justify-between gap-2">
      <span className={`text-[11.5px] ${blocker ? (blockerIsDanger(blocker) ? "text-danger" : "text-text-tertiary") : "text-text-tertiary"}`}
        aria-live="polite">
        {blocker ?? readyLabel(readinessInput)}
      </span>
      <div className="flex items-center gap-2">
        {/* Cancel button — unchanged */}
        <button onClick={handleSave} disabled={saving || blocker !== null || antigravitySaveBlocked} className="…unchanged…">
          {/* label logic unchanged */}
        </button>
      </div>
    </div>
  </div>
</div>
```

Section separators (canon rule 9): the scroll container uses `px-6 py-5` with NO `space-y`; every `Section` after the first carries `mt-5 border-t border-overlay/[0.06] pt-5`. Implement by giving `Section` an optional `first?: boolean` prop (Identity passes `first`) that omits those classes.

- [ ] **Step 3: Remove dead code**

Delete now-unused imports in `Builder.tsx` (`MessageSquare`, `Waypoints`, `Terminal`, role icons, `Plus`, etc. — `pnpm build` reports each unused import). Delete `roleLook`/`BUILTIN_ROLE_LOOKS` if Task 5 moved them.

- [ ] **Step 4: Type-check, screenshot, commit**

Run: `pnpm build` → 0. `pnpm uishot builder` → two-column modal, rail with dots (Identity hollow, others accent), footer `Name required`, disabled primary. `pnpm uishot builder --scenario empty` → same shell with empty skill/role lists rendering without errors.

```bash
git commit -m "feat(builder): two-column shell with section rail, scroll-spy and footer blocker (spec D1/D2/D7)" -- src/components/Builder.tsx
```

---

### Task 8: `builder-edit` fixture view (D11) + docs

**Files:**
- Modify: `src/components/AppShell.tsx:190-210` (the `#view=` map)
- Modify: `scripts/uishot.mjs:17` (usage string)
- Modify: `CLAUDE.md` (view ids line)

- [ ] **Step 1: Add the view**

In the `open` map add:

```ts
"builder-edit": () => {
  // Edit mode with Position: the first fixture definition, scoped to the
  // fixture workspace agent that instantiates it (fixture data only).
  void ipc.agentDef.list().then((defs) => {
    const def = defs[0];
    if (!def) return;
    setBuilderInitialDef(def);
    setShowBuilder(true);
  });
},
```

`selectedId` must resolve to a workspace agent whose `agentDefId === defs[0].id` so `positionEnabled` is true. Find how the fixture boot selects the first roster agent (grep `setSelectedId` in `AppShell.tsx` and `instance.list` in `src/fixtures/scenarios/default.ts`); if the boot-selected agent is not an instance of `defs[0]`, pick the def that matches the selected instance instead:

```ts
const def = defs.find((d) => d.id === selectedInstance?.agentDefId) ?? defs[0];
```

The regex `/view=([a-z-]+)/` already accepts the hyphen.

- [ ] **Step 2: Docs**

`scripts/uishot.mjs` usage: add `builder-edit` to the example view list. `CLAUDE.md` "View ids" line: `home laneboard memory artifacts blackboard chat library builder builder-edit browser settings`.

- [ ] **Step 3: Verify readiness sentinel**

The sentinel fires after boot regardless of the async `agentDef.list`; if the shot lands before the Builder mounts, move `document.body.dataset.conclaveReady = "1"` for this view behind the `.then` — but first check whether the fixture `agentDef.list` resolves synchronously in the same microtask (fixture handlers return plain values; `call()` wraps them in `Promise.resolve`), in which case the double-rAF already covers it.

Run: `pnpm uishot builder-edit` → open PNG: header `Edit agent`, five rail items including Position, footer `Ready to save`, primary `Save changes`, Advanced auto-open only if the fixture def has custom args.

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(uishot): builder-edit fixture view for the Edit agent modal (spec D11)" -- src/components/AppShell.tsx scripts/uishot.mjs CLAUDE.md
```

---

### Task 9: Pixel gate and READY

- [ ] **Step 1: Kill foreign dev servers**

Run: `lsof -nP -iTCP:1420 -sTCP:LISTEN` → if the listed process's cwd is not this worktree, kill it.

- [ ] **Step 2: Record the gates**

```bash
conclave task gate <ws> agent-builder-redesign -- pnpm build
conclave task gate <ws> agent-builder-redesign -- pnpm uishot builder
conclave task gate <ws> agent-builder-redesign -- pnpm uishot builder --scenario empty
conclave task gate <ws> agent-builder-redesign -- pnpm uishot builder-edit
```

- [ ] **Step 3: Open every PNG** (`.shots/builder-default.png`, `.shots/builder-empty.png`, `.shots/builder-edit-default.png`) with the image reader and compare against the canon artboards `new-empty`, `edit-position`. List every visible deviation in the READY note; a deviation the canon requires but the plan omitted is a `task challenge` to Detoro, not a silent fix.

- [ ] **Step 4: READY note**

`conclave task note <ws> agent-builder-redesign "READY: <last commit SHA>; shots: <three paths>; deviations: <none | list>; canon SHA <pinned>"` then `conclave task state <ws> agent-builder-redesign review`.

---

## Risk ledger

- **Moved closures drift.** Tasks 5–6 copy onClick bodies into props; the original body always wins. Reviewer: diff each closure against `git show d90b779:src/components/Builder.tsx`.
- **`agentType` setter removal.** `handleSave` sends `type: agentType`; keep the value `"cli"`. Dropping the state entirely would change the payload type — do not.
- **Scroll-spy in uishot.** The shot is taken at scrollTop 0, so the rail must show `identity` active; if `useScrollSpy` picks another item at rest, the threshold math is wrong.
- **IntersectionObserver in a hidden webview.** rAF never ticks in a hidden Conclave webview (memory: conclave-browser-click-exit0-raf-hidden); the hook also listens to `scroll`, so highlight still updates on user scroll.
- **Legacy NULL permission mode.** `permissionModeDirty` semantics move with the Permission row; `selectCliKind` stays in the shell and is passed down, so the dirty flag path is unchanged.
- **Custom env is Claude-only.** The Advanced disclosure must keep the `isClaudeCode` gate around the env block; Codex/Antigravity show only Custom args.

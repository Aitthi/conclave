# Plan: Design-pane fixes — narrow composer, agent focus sync, Artifacts in the canvas slot

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro) · authority: in-loop
date: 2026-07-05 (evening) · requester: human (screenshot of the Design view right pane)

## Human request (verbatim intent)

1. The chat input (StdinBar) in the Design view's narrow right pane is broken —
   the placeholder wraps to 4 lines and the composer box balloons.
2. Selecting a different agent while in the Design view desyncs state: after
   closing Design, the app is on the pre-change agent while the input shows /
   targets the newly chosen one.
3. Artifacts should use the same layout as the Design view — canvas-left +
   agent terminal (chat) right, full-window — instead of the current full-page
   center screen.

## Root causes (verified by reading the code, 2026-07-05)

- (1) `StdinBar.tsx:177` placeholder is ~55 chars; in Design mode the terminal
  column is `w-[420px]` (`WorkspacePane.tsx:352`) leaving the textarea ~180px.
  The placeholder wraps, and `autogrow()` (`StdinBar.tsx:153`) pins the box to
  `scrollHeight`, which in Blink/WebKit INCLUDES the rendered placeholder height
  even when the value is empty → the ballooned box in the screenshot.
- (2) One-way focus sync. AppShell owns `selectedId` (Roster) → passed as
  `focusInstanceId`; WorkspacePane owns `activeInstanceId` (tab strip). A tab
  click in Design mode (`WorkspacePane.tsx:368`) never reports back up, so
  `selectedId` goes stale. Worse, the honor-focus effect
  (`WorkspacePane.tsx:189-194`) depends on `[focusInstanceId, tabs]`, and `tabs`
  gets a NEW array identity on every `session:status` event
  (`WorkspacePane.tsx:246-259` maps to a fresh array) → the effect re-fires and
  SNAPS `activeInstanceId` back to the stale `selectedId` — the "agent before
  the change" the human saw.
- (3) `ArtifactsView` is a center-pane destination (`AppShell.tsx:411-417`,
  in `centerScreenOpen` at `AppShell.tsx:188-189`), full-page, replacing the
  WorkspacePane — no terminal beside it.

## Decisions (settled — do not re-open; challenge via `task challenge` with evidence)

- **D1 — StdinBar becomes container-width-aware, not mode-aware.** Measure the
  composer wrapper with a ResizeObserver; below a width threshold the
  placeholder shortens to `"Message the agent…"` and the full hint moves to a
  `title` attribute. StdinBar must NOT receive a "design mode" prop — width is
  the truth, and this fixes every future narrow context for free.
  REJECTED: passing a `narrow` prop from WorkspacePane (couples an input widget
  to shell layout state); CSS-only fix (placeholder text cannot be swapped or
  reliably truncated cross-engine for a wrapping textarea placeholder).
- **D2 — Bidirectional focus sync + churn guard.** WorkspacePane gains optional
  `onActiveInstanceChange?: (id: string) => void`, called on user tab clicks;
  AppShell wires it to `setSelectedId`. The honor-focus effect gains a
  last-applied ref so it applies a `focusInstanceId` at most once — `tabs`
  identity churn can never re-apply a stale focus again.
  REJECTED: removing `tabs` from the effect deps (the effect legitimately needs
  to re-check when tabs finish loading); lifting `activeInstanceId` fully into
  AppShell (larger refactor, remount semantics risk, not needed for the fix).
- **D3 — Artifacts moves into the canvas slot.** `showArtifacts` leaves
  `centerScreenOpen`; the full-window predicate generalizes to
  `slotFullWindow = (showDesign || showArtifacts) && workspacePaneVisible` and
  replaces `designFullWindow` at every use (titlebar column bgs, Rail wrapper,
  Roster wrapper — `AppShell.tsx:205,227-236,244-256,297-300`). Design and
  Artifacts are mutually exclusive in the slot: opening one clears the other
  (this already matches the ⌘D handler's `setShowArtifacts(false)`).
  ArtifactsView's INTERNAL visuals are unchanged — the human specified layout
  only. REJECTED: keeping Artifacts as a center screen and bolting a terminal
  column onto it (duplicates the slot machinery); embedding a chat widget
  inside ArtifactsView (the terminal column IS the chat, per design-native).
- **D4 — Same latency semantics as Design.** The artifacts flag stays latent
  behind center screens (matches ruled D3 of design-native; checklist r13 item
  [4] documents this as expected). `Roster onSelect` STOPS force-closing
  artifacts (`AppShell.tsx:313` — remove that line) to mirror design's latency;
  `handleSelectWorkspace` keeps clearing both flags. ⌘⇧A becomes the mirror of
  ⌘D: `setShowDesign(false); setShowArtifacts(v => !v);` and no longer touches
  the center-screen flags. Rail's Artifacts button gets the same body.
  AMENDED at implementation (defect found by Tiësto, ruling: upheld): the
  original step-4 enumeration missed the four center-screen OPEN handlers
  (`onOpenBlackboard`/`onOpenChat`/`onOpenMemory`/`onOpenLaneBoard` in the
  Roster props) — each carried a `setShowArtifacts(false)` from the
  center-screen era. D4's principle (Design is never cleared by those
  handlers, artifacts mirrors Design exactly) requires removing all four;
  an enumeration that contradicts its own stated principle is a plan bug.
  Guard for later tasks: when a flag changes CATEGORY (center screen →
  canvas slot), grep every write site of that flag and re-derive each one
  from the new category's rules — don't trust the plan's site list.
- **D5 — ChatRail wrapper gets `inert` + `aria-hidden` when collapsed**
  (`WorkspacePane.tsx:515`). Same hidden-tab-stop defect class as Armin F1 on
  Rail/Roster, already in the recorded follow-up bag, and this lane touches
  that exact line while generalizing it to `slotOpen` — fix it here.

## Lane 1 — `stdin-narrow-composer` (implementer: Dew · reviewer: Mellow)

Boundary: `src/components/StdinBar.tsx`, `src/components/RoutingPicker.tsx`

1. In `StdinBar`, observe the composer wrapper div (the `ref={dropRef}` box at
   `StdinBar.tsx:198`) with a ResizeObserver → `const [narrow, setNarrow] =
   useState(false)`; set `narrow = width < 560` (tune the threshold empirically
   — the full placeholder must never wrap at the boundary; state-guard the
   setter so an unchanged value doesn't re-render). NOTE `useFileDrop` already
   owns `dropRef` — attach the observer via a callback-ref merge or a second
   ref on the same element; do not break the drop highlight.
2. Placeholder for the self-target live-session case becomes:
   narrow → `"Message the agent…"`; wide → the current full string. The other
   two placeholder cases (`Type to inject…`, `No running session`) are short
   already — leave them. Add `title="Enter to send · Shift+Enter for newline"`
   on the textarea so the hint survives in narrow mode.
3. In `RoutingPicker`, make the chip label robust in narrow contexts: the
   agent-name span gets `truncate` + a `max-w-*` cap so a long agent name can
   never push the textarea to zero width. Do not change wide-mode visuals.
4. Manual check (dev): `pnpm dev` in a plain browser is enough for THIS lane —
   render the pane, narrow the window until the composer is < 560px, confirm
   the box stays one line tall with the short placeholder, and that typing a
   long multiline draft still auto-grows and shrinks back after clearing.

Acceptance: in a 420px terminal column the composer is a single-line-height
box with a non-wrapping placeholder; wide mode is pixel-identical to today
except nothing else; no new props on StdinBar; drop-target highlight intact.

Gate (run in this order, commit first):
`conclave stage commit … -m "…"` then
`conclave task gate <ws> stdin-narrow-composer -- sh -c "pnpm exec tsc --noEmit"`

## Lane 2 — `canvas-slot-artifacts` (implementer: Tiësto · reviewer: Mellow)

Boundary: `src/components/AppShell.tsx`, `src/components/WorkspacePane.tsx`,
`src/components/ArtifactsView.tsx`, `src/components/Rail.tsx`

### 2a. Focus sync (fixes human issue 2)

1. `WorkspacePane` props: add `onActiveInstanceChange?: (id: string) => void`.
   Call it in the tab-strip click handler (`WorkspacePane.tsx:368`) alongside
   `setActiveInstanceId`. Do NOT call it from the initial auto-focus or the
   honor-focus effect (the parent already knows those values; syncing the
   auto-pick would also change Roster's no-selection-until-click behavior).
2. Honor-focus effect (`WorkspacePane.tsx:189-194`): add
   `const lastAppliedFocus = useRef<string | null>(null)`; early-return when
   `focusInstanceId === lastAppliedFocus.current`; set the ref when applying.
   Keep `tabs` in the deps (still needed for the late-load case).
3. `AppShell`: pass `onActiveInstanceChange={(id) => setSelectedId(id)}` to
   WorkspacePane (`AppShell.tsx:424-438`).

### 2b. Artifacts in the canvas slot (fixes human issue 3)

4. `AppShell`: remove `showArtifacts` from `centerScreenOpen` (188-189, update
   the rot-guard comment — the canonical list shrinks); rename/generalize
   `designFullWindow` → `slotFullWindow = (showDesign || showArtifacts) &&
   workspacePaneVisible` and update ALL uses (205, 227-236, 244-256, 297-300).
   Delete the `showArtifacts && activeWorkspaceId` center branch (411-417).
   Pass `artifactsOpen={showArtifacts}` +
   `onCloseArtifacts={() => setShowArtifacts(false)}` to WorkspacePane.
   ⌘⇧A handler (147-154) → `setShowDesign(false); setShowArtifacts(v => !v);`
   (drop the four center-screen closes). Rail `onOpenArtifacts` (269-275) →
   same two-line body. Roster `onSelect`: remove `setShowArtifacts(false)`
   (313) per D4. `handleSelectWorkspace` (172-173) unchanged.
5. `WorkspacePane`: add `artifactsOpen`/`onCloseArtifacts` props;
   `const slotOpen = designOpen || artifactsOpen;`. Every `designOpen ?`
   className ternary on the slot div, `<main>` width, and ChatRail wrapper
   (309, 318, 343, 352, 515) switches to `slotOpen`. Slot content:
   `designOpen ? <DesignView…/> : artifactsOpen ? <ArtifactsView…/> : null`
   in BOTH the loaded tree and the loading/empty branch (306-329).
   THE ONE HARD CONSTRAINT of the design-native lane still rules: the slot div,
   `<main>`, and the ChatRail wrapper stay the same three siblings in the same
   order in both modes — only classNames and slot children change; the
   terminal must never remount on any toggle (comment block at 335-342 is the
   law; extend it to mention artifacts).
6. `ArtifactsView`: root element `<main>` → `<section>` (it now renders inside
   WorkspacePane, which already has the page's `<main>`); keep every visual.
   Its `onClose` now lands on `onCloseArtifacts`. It no longer renders
   anywhere as a center screen — delete nothing else inside it.
7. ChatRail wrapper (515): `slotOpen` + add `inert={slotOpen}`
   `aria-hidden={slotOpen || undefined}` (D5).
8. `Rail.tsx`: expected NO change (props already exist); in boundary in case
   the active-state binding needs a touch.

Acceptance: opening Artifacts (⌘⇧A or Rail) shows artifacts-left + 420px
terminal-right with Rail/Roster collapsed, exactly like Design; Design and
Artifacts never render together; closing either restores the 3-pane layout
with no terminal remount (scrollback survives); switching tabs in either mode
then closing keeps the SAME agent active everywhere (tab strip, terminal,
StdinBar chip, Roster highlight).

Gate (commit first, then):
`conclave task gate <ws> canvas-slot-artifacts -- sh -c "pnpm build"`
(`pnpm build` = `tsc && vite build` — covers the type gate.)

## Global constraints (both lanes inherit)

- UI copy in English only (recorded convention).
- Do not touch files outside your boundary; a needed out-of-boundary one-liner
  is a `task challenge` to Detoro (pre-ratification precedent b90bf164), never
  a silent edit.
- Shared checkout: use `conclave stage commit <ws> <slug> -m …` (private
  index), never raw `git add`/`git commit`.
- Commit BEFORE gating — the gate pins `git rev-parse HEAD` at run time.
- Design canon: layout per the human's explicit instruction (this plan) +
  design-native canon @2df138d patterns; visual questions escalate to Arta
  (688719b6…); plan/spec conflicts escalate to Detoro (bfb737ff…) via
  `task challenge`.
- React 19 + StrictMode discipline: effects must survive
  mount→cleanup→mount (see the spawn-effect comment at WorkspacePane:196-204).

## Risk ledger (known fragility — hit it prepared)

- **Sync loop risk (Lane 2):** tab click → `setSelectedId` → `focusInstanceId`
  prop → honor-focus effect → `setActiveInstanceId` (same id, no-op) → stable.
  The ref guard makes a second application impossible. Verify in dev with
  StrictMode double-invoke anyway.
- **`tabs` identity churn is constant** (every `session:status` event maps a
  fresh array) — anything keyed off `tabs` in deps re-fires often; never put a
  state setter with a stale payload inside such an effect.
- **Placeholder-height ballooning** is in `scrollHeight` of the EMPTY textarea
  (Blink includes placeholder). The short placeholder must be applied in the
  same render as the width flips, or the box flashes tall.
- **`useFileDrop` owns the wrapper ref** (Lane 1) — merging refs wrong silently
  kills the drag-drop highlight; test a file drag after the change.
- **WorkspacePane remounts** on `agentsVersion` bump / workspace switch; after
  D2 the remount's initial auto-focus reads the SYNCED `selectedId`, which is
  what preserves the user's choice across remounts — do not "optimize" the
  focusInstanceIdRef away.
- **termTabMode has two modes** ("remount" default, "keep-alive" via
  localStorage): the slot changes must not disturb the cli-tab filter at
  WorkspacePane:410-412; sanity-check both modes for the no-remount guarantee.
- **Full app verification needs a Tauri rebuild** — implementers verify with
  `pnpm dev` (browser, mock-less IPC failures land in the load-error branch;
  layout/class logic is still fully exercisable) + `pnpm build`; the real
  click-through lands on the human's r14 checklist, which Detoro appends at
  close.

## Deferred (recorded, NOT in scope)

- Roster highlight following the initial auto-pick (behavior change, needs a
  design opinion).
- Any ArtifactsView internal redesign for the narrower column (filters row may
  wrap; acceptable this pass — report, don't fix).
- ChatHub "Open chat" flow from inside artifacts mode (unchanged semantics).

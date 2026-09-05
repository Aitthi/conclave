# Supervisor picker footer hierarchy

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop
Implementer: Dabin (21a29434-cfd8-4b6e-a519-4f2a2a29d093). Aoki rules and integrates.
User request: improve the design shown in /Users/detoro/Desktop/Screenshot 2569-09-05 at 12.37.38.png (edit-supervisor footer). This is additive to the public logo task.

## Reading order
PRODUCT.md → src/styles/app.css → src/components/SupervisorPicker.tsx → this plan.

## Design canon and decisions
This is a bounded correction to existing native modal styling, not a new modal. Existing composition stays pinned to current main component; footer amendment supersedes its legacy @d2ac161 placement comments.
- Edit with supervisor: Remove supervisor is a leading left-aligned independent action; flexible space; Cancel then Confirm form the right-aligned action group. Preserve this DOM/tab order.
- Remove supervisor: retain Ban glyph and explicit text; use quiet danger color consistently for glyph AND label, soft danger hover surface, no filled destructive primary. Target 30–32px button height. Keep readable against both themes (>=4.5:1 label contrast; use darker/lighter danger shade locally if token fails).
- Cancel: native neutral secondary button, readable label, clear hover/focus affordance. Confirm remains sole blue primary. Align every button to same vertical center/height; compact 8px group gap, 16px outer inset, 56px footer. Labels must never wrap or overlap at existing 420px modal width (and 880px app minimum).
- Edit without current supervisor: no Remove action, Cancel+Confirm remain right aligned, no empty placeholder button.
- Add variant retains Skip on left and Add agent on right with current disabled/pending behavior.
- Preserve one-click removal onPick(null), Confirm draft value, Cancel onClose, selection/exclusion, pending disabling, error placement and keyboard behavior. No new confirmation flow and no wording/behavior changes outside footer.
- Use existing tokens and component vocabulary; no global CSS or shared primitive changes. Correct misleading placement comments. A small matching design/screens/supervisor-footer.tsx canon is allowed if required by designer workflow, not required as runtime dependency.

## Boundary
src/components/SupervisorPicker.tsx; optional design/screens/supervisor-footer.tsx. Plan is lead-owned. No edits to AppShell, fixtures, global styles, Builder or branding. No package changes. Scratch capture scripts may stay untracked outside commit.

## Execution and gates
Use conclave lane start, isolated worktree. Read user screenshot. Inspect current real picker before edit. Implement exact amendment above. At lane finish use conclave stage commit; no self merge.
Before READY: check lsof -nP -iTCP:1420 -sTCP:LISTEN and server cwd; coordinate via Aoki if another active lane owns it. Run and record conclave task gate <ws> supervisor-footer-hierarchy -- pnpm uishot home, OPEN PNG. Since modal is behind control, ALSO capture actual opened edit picker through fixture browser interaction, dark and light, with/without supervisor, and add variant. Inspect each PNG incl footer at app minimum width. If controls unreachable in fixtures, report concrete blocker; never substitute plain home PNG as proof of changed footer. No empty scenario is necessary unless empty-state code changes. Record pnpm build gate and exact callback/disabled verification (manual browser/component harness is sufficient, no trivial tests). READY note lists screenshot paths, commit SHA, checks and any limitations. Exact phrase: FOOTER READY.

## Risks
The supplied screenshot is cropped; review entire modal to preserve visual balance. Other active work: Hardwell branding assets, Arta Builder canon; neither owns this component. Task state is not acceptance until Aoki has viewed pixels and integrated. Existing fixture tool does not open dialogs by itself; additional interactive capture is mandatory.

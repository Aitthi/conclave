# Overview and workspace management canon

Owner: Hardwell. Product/API ruling and acceptance: Aoki.

## Acceptance split

Archive is independently reviewable. AI usage Overview remains a provisional layout pending the usage data contract. Do not implement its example date range, unknown-only calendar, or legend as final telemetry semantics.

## Archive contract

Source: `docs/plans/2026-09-05-workspace-archive-engine.md`.

`workspace-archive.tsx` is the separate Workspaces management destination. Overview stays at the Conclave brand entry and concerns AI usage. Workspaces and Archived are local list filters; changing them never launches or stops agents.

Archive requires a non-hidden, stopped workspace with no live runtimes or busy one-shot work. Started, including started with zero runtimes, requires explicit Stop first. Stop confirmation preserves the existing destructive-runtime warning; it never automatically archives. A fresh backend refusal stays inline and preserves the row. Hidden workspaces are excluded.

Archive retains agents, availability, sessions, tasks, messages, memory, artifacts and project files. On success remove the row from the active list and Rail, then offer Undo through Restore. Restore returns stopped, preserves availability, launches nothing and keeps the Archived destination selected. Its success message offers explicit Open. Permanent delete remains a distinct, confirmed action accessible through management for either list.

Bind to `workspace.list`, `workspace.listArchived`, `workspace.archive({workspaceId})`, `workspace.restore({workspaceId})` and existing stop/update/delete handlers. Refetch on `workspace:changed`; `archivedAt` and `runState` are independent. Show pending locally, serialize mutations, and preserve the row on failure. The prototype simulates these transitions entirely in local React state.

## Layout and controls

56 px Rail, 64 px header, 20–24 px content padding, compact three-column list with folder path beneath name. Minimum reviewed window 880×560. Long paths truncate with full title text. Settings use a native modal dialog at 500 px maximum width; body scrolls while header/footer stay visible. Native dialog provides focus containment and Escape; pending mutations block dismissal.

This canon concentrates on the new Archive section and management flow. Preserve production EditWorkspace's existing colour/custom-colour picker and its rename behavior when integrating; the specimen omits the unchanged colour picker. Reuse existing workspace colours and global library actions in the production Rail. The specimen's Link folder and Open handlers identify their integration destinations and do not invoke native actions.

## Preview states

Use `?project=1qod8l2&state=STATE&theme=light#/workspace-archive` on the design host; `theme=dark` selects dark mode. States: workspaces, archived, empty, all-archived, archive-empty, search-empty, loading, error, settings-stopped, settings-started, settings-busy, archive-pending, archive-error, restore-pending, restore-error, restored.

## Token translation

| Canon | Production |
| --- | --- |
| bg-canvas | bg-bg-canvas |
| bg-fill | bg-fill-soft |
| border-border / ring-border | border-overlay/[0.06] / ring-overlay/[0.08] |
| text-text-secondary | text-text-secondary |
| surface, surface-raised, accent, danger | Same semantic token names |

No production token files are changed. The Overview's scoped indigo heat ramp is explicitly requested by the human reference; final numeric thresholds require the usage ruling.

## Verification

Opened and inspected list light 1280×800, archived dark 1280×800, settings stopped light, settings started dark, busy dark, restore error light, all archived dark and archive error light at 880×560. No page errors or horizontal document overflow. Settings body scroll is intentional; permanent-delete controls remain reachable above the fixed footer.

Pixel atlas: `design/screens/workspace-archive.png`. Individual captures: `/tmp/archive-workspaces-light.png`, `/tmp/archive-archived-dark.png`, `/tmp/archive-settings-stopped-light.png`, `/tmp/archive-settings-started-dark.png`, `/tmp/archive-settings-busy-dark.png`, `/tmp/archive-restore-error-light.png`, `/tmp/archive-all-archived-dark.png`, `/tmp/archive-archive-error-light.png`.

Browser interaction checks passed: Archive removes the Rail entry; Undo returns it stopped; Restore updates the Rail while keeping Archived selected; Stop confirmation leaves the workspace visible and requires separate Archive; Escape closes settings. Initial check attempts timed out because host :7343 stopped answering HTTP; verification succeeded on :7344, which reads the same main checkout. Design review passed with zero serious findings.

Craft rubric after pixels: philosophy 4, hierarchy 4, execution 4, specificity 4, restraint 4, variety 4. Production integration still requires the real-app UI pixel gate; these are design-host captures only.

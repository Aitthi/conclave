# Overview and workspace management canon

Owner: Hardwell. Product/API ruling and acceptance: Aoki.

## Acceptance split

Archive and AI usage Overview visuals at `f4c94f79` are accepted by Aoki. The final source ruling in `docs/plans/2026-09-05-usage-overview-contract.md` supersedes the specimen's provisional activity helper and model naming as specified below. Geometry stays unchanged. Final frontend wire types and collection enablement remain Aoki's gate. The superseding browser ruling permitted compliant interaction rechecks after navigation committed; those results are recorded below.

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

No production token files are changed. The Overview's scoped indigo heat ramp is explicitly requested by the human reference. Legend bands are 0, 1, 2, 3, 4+ recorded activity records per day; exact counts remain available for every cell.

### Scoped Archive contrast correction

Use component-scoped `--archive-danger-text` in production: light `#b42318`, dark `#ff8a80`. Apply to destructive labels, inline errors and destructive text buttons. Use a 10% mix of that colour with the actual surface for alert backgrounds; do not change global `--color-danger`. Canon scopes its equivalent override under `.archive-canon` only. Reuse the same scoped token on EditWorkspace's Archive/error portion; retain normal button contrast rules for filled destructive confirmation controls.

Measured WCAG sRGB contrast: light text on white 6.57:1; light 10% alert tint 5.57:1; dark text on #232325 6.87:1; dark 10% alert tint 5.77:1. Browser computed foreground and colour-mix values matched the calculation. Final light/dark restore-alert captures were reopened after correction. This supersedes the first atlas's bright error red.

## Usage presentation contract

Default is 90 calendar dates including today, with a 30-day alternative. The specimen freezes 5 Sep 2026 in Asia/Bangkok and labels all six example records as illustrative. Production computes calendar date buckets from the browser IANA timezone using DST-aware half-open UTC boundaries; never lift the specimen's fixed UTC date arithmetic into collection or production bucketing.

Model, agent and workspace selectors apply equally to measured token figures, recorded activity, attribution rows and heatmap. Provider may appear as metadata; no independent provider filter. All workspace scope includes archived history, labelled Archived; hidden internal workspaces are excluded. Response-observed models carry an always-visible **Reported** badge. A model known only from selection, including Codex turn_context, carries an always-visible **Selected** badge. If neither basis is known, display **Unknown model**. Never substitute a selected name without its basis label.

Model identity is the stable tuple of provider, name and basis. Selected and Reported remain separate keys even when names match. Use the same keys and visible basis across filter options, model rows and agent/workspace breakdown details. Keep requested_model and served_model separate in the adapter. Latest context uses the configured model with Selected basis. These labels clarify source evidence; they must not imply that a selected model was observed serving a response.

Measured tokens aggregate input plus output only for records with both known. Cache subsets are not added again. Pair the subtotal with count of records missing token usage. If all records lack usage, show an em dash; do not manufacture a zero subtotal.

The activity headline remains **Recorded activity**. Its exact helper text is **Model responses and draft runs**. One activity is either one completed model-response record or one successful draft invocation. Carry activityCount, responseCount and invocationCount in every summary, day and breakdown; activityCount equals the two split counts combined. Show the response/draft split in summary and breakdown secondary text, and expose both exact counts in daily hover/focus details and the daily table. Example: “6 recorded activities · 5 model responses · 1 draft run”. Never describe this count as user turns or API requests. This wording supersedes “Completed responses / stable usage records” in the accepted visual specimen without altering its layout.

Successful drafts with unknown terminal token usage still count as draft runs and contribute to the missing-token-record count. Failed, cancelled and incomplete direct streams contribute no completed activity. Direct chat/fusion contributes its terminal provider response without an enclosing fusion-run duplicate. One-shot invocation aggregates must not overlap imported transcript responses. Multi-iteration/fallback totals retain only the verified top-level aggregate once; if attribution to a reported model is unproven, preserve Selected basis and partial attribution coverage instead of assigning the total to one reported model.

Coverage is complete/partial/none, returned by the data adapter for the exact filtered scope and period. A complete empty day is 0; a partial empty day is an em dash; no coverage is Unavailable. Partial observed records retain the exact count with a marker. Earlier history before collection and outages remain unknown. Outside-range calendar slots are blank. Today is labelled in progress. The deterministic specimen includes all these distinctions; its coverage dates are fixture data, not source policy.

Heatmap: seven weekday rows, chronological week columns, month/date labels, 16 px rounded cells with 6 px gaps, scoped indigo intensity ramp. Hover/focus shows date, exact activity count, responseCount, invocationCount, coverage and current-day status. Roving tab stop and arrow keys move within the chart; key propagation stops so design-host navigation cannot hijack chart arrows. Daily table exposes date, recorded activity, model responses, draft runs and coverage without colour reliance. Legend intensity remains the combined recorded activity count per day.

Context is a separate latest-per-agent gauge. Show used/capacity tokens, source and observation time; stale observations retain their age label and unknown remains unavailable. Never sum context snapshots or combine them with period usage. Date range does not change current context; identity filters do. The prototype's context examples are independently labelled illustrative data.

Usage preview states: default/partial, empty (no events with gaps), zero (verified empty historical dates with today partial), none, unsupported, loading, error. Filters exercise Unknown model/all-missing-token rows. Initial source coverage and history have no assumed backfill.

Usage evidence: `design/screens/workspace-overview.png` atlas. Full light/dark captures `/tmp/usage-canon-light.png` and `/tmp/usage-canon-dark.png`; 880×560 state captures `/tmp/usage-min-default-light.png`, `/tmp/usage-min-loading-dark.png`, `/tmp/usage-min-error-light.png`, `/tmp/usage-min-none-dark.png`, `/tmp/usage-min-empty-light.png`. Context was separately opened at `/tmp/usage-context-dark.png`, then recaptured with the final grid in full captures. Filter check: Unknown model gives one activity, one missing token record, and em dash measured usage. Keyboard check: Sep 5 Left moves to Aug 29. Daily-table toggle exposes the calendar table. Fresh main-host :7343 final captures supersede intermediate :7344 captures whose bundled CSS omitted some generated utilities.

## Verification

Opened and inspected list light 1280×800, archived dark 1280×800, settings stopped light, settings started dark, busy dark, restore error light, all archived dark and archive error light at 880×560. No page errors or horizontal document overflow. Settings body scroll is intentional; permanent-delete controls remain reachable above the fixed footer.

Pixel atlas: `design/screens/workspace-archive.png`. Individual captures: `/tmp/archive-workspaces-light.png`, `/tmp/archive-archived-dark.png`, `/tmp/archive-settings-stopped-light.png`, `/tmp/archive-settings-started-dark.png`, `/tmp/archive-settings-busy-dark.png`, `/tmp/archive-restore-error-light.png`, `/tmp/archive-all-archived-dark.png`, `/tmp/archive-archive-error-light.png`.

Historical standalone browser checks exercised Archive/Undo/Restore, separate Stop and Archive, Escape, filters and calendar navigation. These are not accepted protocol interaction evidence: gate 1b30daac used standalone Puppeteer in conflict with `protocol:browser-conclave-only`. An initial compliant snapshot failed before navigation committed (task note d70762da). Detoro subsequently permitted rechecks after actual navigation commit; open.loading=false is not readiness evidence.

On 2026-09-05, all superseding interaction checks used the owned `conclave browser` tab exclusively. After opening and allowing navigation to commit, snapshot returned readyState=complete. The Daily table click returned ok:true and exposed Sep 1 = 0/complete, Sep 3 = 1/partial, earlier dates = unavailable/none. The model selector changed through local browser eval to Unknown model; observed result was one activity record, one unknown-token record and em dash Measured tokens. Archive workspace and Undo clicks both returned ok:true. Snapshots showed northstar removed from the active list/Rail (counts 1 active/3 archived), then restored stopped (counts 2/2) with explicit Open feedback. No screenshots or fallback tools were used in this pass. `conclave browser close` confirmed activeTabId=null and tabs=[]. These checks supersede the corresponding standalone checks; they do not retroactively validate the standalone gate or claim a fresh real-keyboard test. Visual design/contrast acceptance remains valid; design review passed with zero serious findings.

Craft rubric after pixels: philosophy 4, hierarchy 4, execution 4, specificity 4, restraint 4, variety 4. Production integration still requires the real-app UI pixel gate; these are design-host captures only.

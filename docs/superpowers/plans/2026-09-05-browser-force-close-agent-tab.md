# In-app browser: human can force-close a live agent tab

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
implementer: Dabin (21a29434) · reviewer: Armin (be81029a) · escalation: Detoro via `task challenge`
base: main @ f65f391 · boundary: `src/components/InAppBrowserView.tsx`, `src/fixtures/scenarios/default.ts`

## Request (human, 2026-09-05)

> เพิ่มให้ force ปิด tab ที่ agent เปิดขึ้นมาได้ คือ agent ใช้แล้วไม่ยอมปิด ทุกทีทำให้ใช้ไปสัก ram เต็ม

Agents open tabs through `conclave browser open` and never call `browser close`; every live agent tab keeps a native webview alive, and the human currently has no way to reclaim them (a live agent tab shows no ✕ — `InAppBrowserView.tsx:420-426`).

Design canon: `docs/superpowers/specs/2026-07-11-inapp-browser-multitab-redesign-design.md` — **D2-amend (2026-09-05)** in the decision table is the ruling this lane implements. The existing hover-✕ on ended agent tabs (`TabRow`, `InAppBrowserView.tsx:85-179`) is the visual pattern to reuse.

## Decisions (Detoro rulings — final)

1. **Every agent tab is closable by the human, live or ended.** `TabRow` receives `onClose` for all agent rows, not only `t.ended`. The backend already accepts `browser.close {tabId}` for any tab (`commands/browser.rs:312-334`, `runtime/browser.rs::close_tab`) — no Rust change.
2. **Live tabs get a distinct affordance so a force-close is a deliberate act, not a slip.** Live agent row ✕: `title="Force close — the agent is still using this tab"`, `aria-label="Force close <label>'s tab"`, danger hover colour (`hover:text-danger hover:bg-danger/[0.08]`). Ended rows keep today's neutral ✕ and labels. No confirm dialog: the agent recreates its tab on its next `open`/`goto` (D2-amend), so the action is recoverable.
3. **Section header gains "Close all"** for the agent section when `agentTabs.length > 0`: a small text button right-aligned in the `SectionLabel` row, `aria-label="Close all agent tabs"`, which calls `browser.close` for each agent tab sequentially (await each; stop on first error and surface the existing `setError` string). This is what actually solves the RAM complaint when ten agents have leaked ten tabs; per-row ✕ alone would be ten clicks.
4. **Empty-state copy** (`InAppBrowserView.tsx:~440`): change "its session shows up in the rail as a read-only tab" → "its session shows up in the rail as a read-only tab you can close at any time". One sentence, English (memory: UI copy is English).
5. **Fixture**: `src/fixtures/scenarios/default.ts:35-60` already seeds one live agent tab (`ended: false`) and one ended agent tab (`ended: true`), so both ✕ variants render in the pixel gate without new data. Touch the fixture only if the Close-all flow needs a handler tweak (fixed literal timestamps only).

## Gates (record each with `conclave task gate <ws> browser-force-close-agent-tab -- <cmd>`)

1. `pnpm build` exit 0.
2. `pnpm uishot browser` and `pnpm uishot browser --scenario empty` exit 0, then **Read both PNGs** (`.shots/browser-default.png`, `.shots/browser-empty.png`). Hover state cannot be captured by uishot; verify the ✕ variants with `conclave browser`: `goto http://localhost:1420/?fixture=default#view=browser`, `eval` that `[aria-label^="Force close"]` count equals the number of live agent tabs and `[aria-label="Close all agent tabs"]` exists, `click` the Close-all button, `eval` that the agent section is gone, `screenshot` under `.shots/`, Read it. Kill any foreign vite on :1420 first (CLAUDE.md).
3. Human acceptance after rebuild+relaunch: with a live agent tab in the rail, hover → red ✕ → click → tab gone, RAM drops (Activity Monitor: one fewer WebContent process). Agent's next `conclave browser open` recreates the tab.

## Risks

- `doClose` guards on `busy`; Close-all must run inside one busy span, not fire N parallel `doClose` calls that each early-return.
- Closing the ACTIVE tab makes the backend reselect the first remaining tab and show it; the view's `applyState` from the close response handles this today.
- The uishot console-fail rule: any `[fixture]` message fails the shot — add fixture handlers, never swallow.

## Deferred

- Auto-close on agent end — still rejected (D4b).
- LRU eviction — v2 per the design spec §9.
- A hint in the agent-facing Tool Map skill text reminding agents to `conclave browser close` when done — sidecar file, outside the repo; lead follows up separately.

## Outcome

_(implementer fills: commits, gate ids, shot paths, deviations)_

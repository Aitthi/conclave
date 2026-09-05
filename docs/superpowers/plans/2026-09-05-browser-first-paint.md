# In-app browser: first navigation paints without a tab switch

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
implementer: Tiësto (e60b9644) · reviewer: Armin (be81029a) · escalation: Detoro via `task challenge`
base: main @ f65f391 · boundary: `src-tauri/src/engine/runtime/browser.rs`, `src-tauri/src/engine/runtime/browser_tabs.rs`

## Bug (human, 2026-09-05)

> ระบบ browser เปิด web ครั้งแรกจะไม่ขึ้น ต้องสลับ tab ไปมา

The first page opened in the in-app browser stays blank until the human switches to another tab and back.

## Root cause (Detoro, confirmed by reading — not yet reproduced live; implementer reproduces first)

`runtime/browser.rs::navigate` (~line 518) on the CREATE branch calls `window.add_child(...)` at `resolve_bounds(bounds)` — offscreen when `bounds` is `None` — and then unconditionally `view.hide()`. Nothing after that shows or positions the webview:

- Human flow: `doNewTab` → `newTab` + `setActive` (registry only, no webview yet, so `set_active` is a no-op) → types a URL → `doGoto` → `browser.goto {tabId,url}` → `navigate(.., bounds: None)` creates the webview hidden + offscreen. `InAppBrowserView.tsx:319-330` then only `applyState`s. The webview first becomes visible on the next `set_active` (a tab switch), which is exactly the workaround the human found.
- Agent flow on an empty rail while the Browser view is mounted: `browser.open` from an agent → `upsert` makes the first-ever tab active → webview created hidden + offscreen; the 2s poll sees the tab but `doSelect` early-returns on `tabId === activeTabId`, so nothing shows it.

`set_visible`/`set_bounds` only act on the active tab's webview *at call time*, and the mount-time calls in `InAppBrowserView.tsx:262-285` run before any webview exists.

## Decisions (Detoro rulings — final)

1. **Fix lives in the backend, not the React view.** The registry becomes the single source of truth for overlay visibility and the last reported bounds, so every path that creates or reveals a webview lands it at the right place. A frontend-only fix (call `setActive`+`syncBounds` after `goto`) would leave the agent-on-empty-rail case broken and would keep the create/show ordering split across two processes.
2. **`TabRegistry` gains two fields** (`browser_tabs.rs`, pure, unit-tested): `overlay_visible: bool` (set by `set_visible`) and `last_bounds: Option<Bounds>` (set by `set_bounds`). `Bounds` is defined at `browser.rs:76`; move it to `browser_tabs.rs` and re-export from `browser.rs` so `commands/browser.rs` keeps compiling unchanged.
3. **`navigate` CREATE branch**: create at `bounds.or(registry.last_bounds)`; after `add_child`, if `tab_id == active_tab_id && overlay_visible` then `show()`, else `hide()`. Never show an inactive tab (invariant: only the active tab is visible — `set_active` doc comment).
4. **`set_active`**: after hiding the others and before `show()`, apply `last_bounds` to the incoming webview when present. Inactive webviews keep stale bounds today; a window resize while on another tab currently relies on the frontend calling `syncBounds` after `doSelect`. Backend-applying makes the invariant hold regardless of caller (CLI `set_active` too).
5. **`set_visible(true)`** applies `last_bounds` to the active webview before showing it, for the same reason. `set_visible(false)` unchanged.
6. **No frontend edits.** (see below)
7. **Every `show()` is gated on `overlay_visible` — amendment 2026-09-05 (Tiësto challenge a4e3a72b + the human's screenshot** `docs/superpowers/plans/assets/2026-09-05-browser-overlay-covers-terminal.png`): an agent's tab painted over the terminal view while the Browser view was NOT mounted. Path: `set_visible(false)` on unmount hides only the tab that was active at that moment; a later `close_tab` reselects another tab and calls `view.show()` unconditionally (`browser.rs:~636`), and `set_active` likewise shows whatever it is given. Rule: `set_active` and `close_tab` (reselect) position the incoming webview at `last_bounds` and `show()` it ONLY when `overlay_visible`; when the overlay is hidden they only update the registry and keep every webview hidden. `set_visible(true)` then reveals the current active tab. Tests: `set_active_does_not_show_when_overlay_hidden`, `close_tab_reselect_does_not_show_when_overlay_hidden`. Second human-acceptance step: with the Browser view closed, run `conclave browser open` + `conclave browser close` as an agent while another agent tab exists — nothing may paint over the terminal.

 If the implementer finds the frontend must change to make the live repro pass, that is a `task challenge` with the evidence, not a silent boundary widening.

## Gates (record each with `conclave task gate <ws> browser-first-paint -- <cmd>`)

1. **Failing test first.** A live GUI repro is not available to an implementer (memory: `pnpm tauri dev` cannot take `conclave.sock`; GUI e2e is the human after rebuild+relaunch), so the root cause above is accepted on reading. Write the failing registry unit test first (create-while-active-and-visible must decide `show` at `last_bounds`), watch it fail, then fix.
2. `cargo test -p conclave browser` (lib target is `conclave_lib`; run from `src-tauri/`) exit 0, including new tests: `navigate_create_shows_when_active_and_overlay_visible`, `navigate_create_hides_when_inactive`, `set_active_applies_last_bounds`, `set_visible_true_applies_last_bounds`. The native `add_child`/`show` calls are not exercisable from `cargo test` (memory: `current_exe()`-style split) — split the decision ("should show / which bounds") into a pure function on the registry and test that; the native wrapper stays thin.
3. `cargo fmt --check` scoped to the two boundary files only (memory: fmt drift on main is pre-existing; never bare `cargo fmt`).
4. `cargo clippy -p <crate> --all-targets` no NEW warnings in the boundary files.
5. Human acceptance after rebuild+relaunch: open Browser view → New tab → type a URL → Enter. **Pass:** the page paints immediately without any tab switch. Then, with an empty rail, run `conclave browser open https://example.com` as an agent while the Browser view is open. **Pass:** the page paints when the tab appears in the rail.

## Risks

- `Webview::set_position`/`set_size` on a hidden webview: verify it is honoured on macOS wry before relying on it (a hidden NSView keeps its frame, so it should be fine; test on the machine).
- `show()` before the first `set_position` flashes the page at the offscreen default — the ordering in decision 3 (position first, then show) matters.
- The registry `Mutex` is held inside `with_registry` closures; do not call native webview methods while holding it (existing code pattern — keep it).

## Deferred

- Load-complete signal (`loading` never truly tracked) — unchanged.
- LRU eviction of idle webviews — see the multitab design spec §9; a separate lane (`browser-force-close-agent-tab`) gives the human a manual close instead.

## Outcome

_(implementer fills: commits, gate ids, deviations)_

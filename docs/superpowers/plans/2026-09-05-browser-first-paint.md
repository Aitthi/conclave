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

Implemented by Tiësto on `lane/browser-first-paint`, base `201b814` (the plan
header says `f65f391`; the lane was branched from `201b814`, the head of `main`
at claim time).

**Commits**

- `2e4b544` — `test(browser)`: the five failing placement tests plus the registry
  surface they exercise, still encoding today's behavior (gate 1's red).
- `a679e3f` — `fix(browser)`: the registry decisions and the native pool wired to
  them.
- `90162d7` — `fix(browser)`: Decision 7 (reveals gated on `overlay_visible`) plus
  Armin's `reveal_at` finding.

**Gates**

| # | Command | Result |
|---|---|---|
| 1 | `cargo test -p conclave --lib browser` @ `2e4b544` | exit 101 — 5 failed, the intended red |
| 2 | `cargo test -p conclave --lib browser` @ `a679e3f` | exit 0 — 53 passed |
| 2b | `cargo test -p conclave --lib` @ `a679e3f` | exit 0 — 1029 passed, 11 ignored |
| 3 | `rustfmt --edition 2021 --check` on both boundary files @ `a679e3f` | exit 0 |
| 4 | `cargo clippy -p conclave --all-targets` @ `a679e3f` | exit 0 — 2 warnings, both pre-existing (`instance.rs:2083`, `pty.rs:230`), zero in the boundary files |
| 2 | `cargo test -p conclave --lib browser` @ `90162d7` | exit 0 — 56 passed |
| 2b | `cargo test -p conclave --lib` @ `90162d7` | exit 0 — 1032 passed, 11 ignored |
| 3 | `rustfmt --edition 2021 --check` on both boundary files @ `90162d7` | exit 0 |
| 4 | `cargo clippy -p conclave --all-targets` @ `90162d7` | exit 0 — same 2 pre-existing warnings, zero in the boundary files |
| 5 | Human acceptance after rebuild + relaunch | pending |

All four named tests exist: `navigate_create_shows_when_active_and_overlay_visible`,
`navigate_create_hides_when_inactive`, `set_active_applies_last_bounds`,
`set_visible_true_applies_last_bounds` — plus four more covering the
agent-on-empty-rail create, the overlay-off-screen create, caller-rect
precedence, and the unknown-id no-op.

**Round 2 — Decision 7 and Armin's review finding (`90162d7`)**

Challenge `a4e3a72b` (the create path was gated on `overlay_visible`, the two
reveal paths were not) was ruled ACCEPTED and amended in as Decision 7; the human
had hit it live at 10:51 (`assets/2026-09-05-browser-overlay-covers-terminal.png`).

- `set_active` and `close_tab`'s reselect now position at `last_bounds` and show
  ONLY when `overlay_visible`. Otherwise the activation is registry-only: the
  active pointer moves and no webview is touched at all, since a reveal is the
  only thing that ever shows one. `set_visible(true)` still reveals the active
  tab, reading its own decision back off the registry.
- `Activation` carries a `Placement` for the incoming tab and `active_reveal`
  became `active_placement`, so "where does it land" and "may it paint" stay one
  decision made in one place.
- Armin's finding: a reveal must never `show()` after a failed bounds
  application, or the page paints at whatever frame it was holding. All three
  reveals funnel through `reveal_at(view, bounds)` — position, then show,
  propagating either failure. `close_tab` previously swallowed both with `let _ =`
  and showed anyway; it now reports, because the tab is already gone and an `Ok`
  would describe a screen the human is not looking at.
- Tests: `set_active_does_not_show_when_overlay_hidden` and
  `close_tab_reselect_does_not_show_when_overlay_hidden` per the ruling, plus
  `close_tab_reselect_shows_when_overlay_visible` for the other arm.
  `reveal_at` itself is native-only and so is not unit-testable here — same seam
  limit gate 2 already names for `add_child`/`show`.

**Deviations from the plan**

1. **`close_tab`'s reselect also repositions before showing** (the plan enumerates
   `navigate`, `set_active`, `set_visible` only). Decision 1 says the registry is
   the source of truth "so every path that creates or reveals a webview lands it
   at the right place", and the reselect after a close is such a path — the
   promoted tab has been hidden and may hold a pre-resize frame. Implementation
   judgment inside the plan's intent; positioning only, no change to what shows.
2. **Gate 3 runs `rustfmt --check` on the two files, not `cargo fmt --check`.**
   `cargo fmt -- --check <paths>` ignores the path arguments and formats the whole
   workspace, which surfaces the pre-existing drift on `main` (`crates/codeintel/*`).
   `rustfmt --edition 2021 --check <paths>` is the scoping the gate asked for.
3. **`navigate`'s create-show was left as `let _ =`.** The `reveal_at` ruling
   names `close_tab`/`set_active`/`set_visible(true)`. `navigate` has no separate
   bounds application to fail — `add_child(.., position, size)` places the webview
   and its failure already propagates before any `show()` — so it already
   satisfies "never show after a failed bounds application".
4. **No `overlay_visible()` / `last_bounds()` accessors.** They were written and
   then removed: nothing outside the registry reads them (the three decision
   functions read the fields directly), and dead code is a `dead_code` warning,
   which gate 4 forbids in the boundary files.

**Not deviated:** no frontend edits (decision 6). The React view already reports
both facts at mount — `setVisible({visible:true})` then `syncBounds()`
(`InAppBrowserView.tsx:262-266`) — and `doGoto` sends `{tabId, url}` with no
rect, which is exactly the `bounds: None` case the registry now backfills from
`last_bounds`.

**Verification limits.** No live GUI repro was run — per the plan's gate 1 and the
recorded environment constraint, `pnpm tauri dev` cannot take `conclave.sock`, so
GUI e2e is the human's after a rebuild + relaunch. The root cause was accepted on
reading and pinned at the registry level; gate 5 remains the real acceptance.

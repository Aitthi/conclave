# In-App Browser: Embed as a Tab, Not a Separate Window

**Date:** 2026-07-09
**Status:** Approved design
**Topic:** Change the Conclave in-app agent browser from a standalone Tauri
`WebviewWindow` (a separate OS window) to a webview embedded inside the main
app window, shown within the existing **Browser** center-pane tab.

## Problem

The V1 in-app browser (commit `01fbb52`) runs the page in its own OS window
(`WebviewWindow`, label `agent-browser`). The `InAppBrowserView` center-pane
tab is only a *control surface* — a URL bar plus a DOM/text snapshot inspector —
because the human can't see the real page (it's a different window). This is
awkward: the human juggles two windows, and the tab duplicates in a text list
what the separate window already shows in pixels.

We want the real page to render **inside the Browser tab** of the main window.

## Goals

- The live page renders embedded in the Browser tab's center-pane region.
- Switching to another tab hides the page but keeps it loaded in the
  background, so an agent can keep driving it (`goto/click/type/snapshot/eval`).
- The Browser tab UI is just a URL bar + toolbar over a full-area page; the DOM
  snapshot inspector is removed from the UI (agents still get snapshots via the
  `browser snapshot` CLI/IPC tool — unchanged).
- When an agent opens/navigates the browser while the human is on another tab,
  the page loads silently in the background and the Rail's browser icon shows a
  dot indicator that a browser is open.

## Non-Goals

- **Pixel screenshot / capture** — tracked as a separate task. This design does
  not add `browser screenshot`.
- **Layering the native webview under React overlays** — a native child webview
  composites *on top of* the DOM; making arbitrary React modals appear above it
  is out of scope (see Risks).
- No change to the agent-facing tool surface semantics (`browser open|goto|
  status|snapshot|click|type|eval|close` behave as today).
- No CDP/Playwright dependency; no cookie/credential export. `browser eval`
  stays a same-user local-only escape hatch.

## Chosen Approach: Native Child Webview

Tauri v2 (2.11.3) hosts multiple webviews in one window. Instead of building a
new `WebviewWindow`, add a second `Webview` to the **main** window
(`window.add_child(WebviewBuilder, position, size)`) positioned over the
Browser tab's center-pane rectangle. React reserves an empty region and reports
its bounding rect; the native webview overlays exactly there.

**Why not an `<iframe>`:** an iframe to a remote origin cannot be scripted from
the parent (same-origin policy), and many sites set `X-Frame-Options`/CSP that
forbid being framed at all. That breaks the whole agent-driving model — every
`browser eval/click/type/snapshot` relies on same-process JS injection an
iframe forbids. The native child webview keeps the entire existing tool
surface working with only a `get_webview_window` → `get_webview` change.

## Architecture

### Backend — `src-tauri/src/engine/runtime/browser.rs`

The label constant stays `agent-browser`. One embedded webview per app process
(V1, unchanged).

- **`open(app, url, bounds?)`** — resolve/normalize URL (unchanged helper). If
  the webview exists, `navigate` + show it (unchanged). If not, resolve the
  main window (`app.get_webview_window("main")`, hard-error if absent) and
  `main.add_child(WebviewBuilder::new(BROWSER_LABEL, WebviewUrl::External(url)),
  position, size)`. If `bounds` is absent at open time, place it hidden /
  offscreen and rely on the first `set_bounds` from React to position it.
- **`goto/status/snapshot/click/type_text/eval_json/close`** — identical logic;
  only the lookup changes from `app.get_webview_window(BROWSER_LABEL)` to
  `app.get_webview(BROWSER_LABEL)`. `eval_value` uses the same
  `eval_with_callback` → oneshot → 10s-timeout bridge (the `Webview` type
  exposes it). `close` calls `webview.close()`.
- **`set_bounds(app, x, y, w, h)`** — new: `webview.set_position(LogicalPosition)`
  + `webview.set_size(LogicalSize)`. Called by React whenever the reserved
  region's size/position changes.
- **`set_visible(app, visible)`** — new: `webview.hide()` / `webview.show()`.
  Hiding does NOT close the webview — the page stays loaded and an agent can
  keep driving it. No-op (graceful `ok`) when no browser is open.

`state_from` still reads `url()`/`title()` off the webview for the status line.

### Backend — `src-tauri/src/engine/commands/browser.rs` + `router.rs`

- Parse `bounds` (optional) on the `browser.open` payload; add two router arms:
  `browser.setBounds` (x,y,width,height) and `browser.setVisible` (visible).
- Map `BrowserError` → `AppError` as today. These two new commands are
  **UI-only plumbing** and are intentionally NOT added to the `conclave` CLI
  verb map (an agent never positions the human's viewport).

### Frontend — `src/components/InAppBrowserView.tsx`

- Remove the DOM/text snapshot inspector section and its `snapshot` fetch.
  Keep the URL bar + toolbar (open/goto/reload/close, status line, error line).
- Below the toolbar, render an empty `<div ref={regionRef}>` that fills the
  remaining area — this is the hole the native webview overlays.
- A `ResizeObserver` on `regionRef` (plus a window-resize listener) computes the
  region's viewport bounding rect and calls `ipc.browser.setBounds({x,y,width,
  height})` (debounced to animation frames).
- On mount / when the tab becomes visible: `ipc.browser.setVisible({visible:
  true})` then push current bounds. On unmount / tab switch away:
  `ipc.browser.setVisible({visible: false})` — never `close`.
- Fixture mode: `setBounds`/`setVisible` are no-op fixture handlers (fixed
  literals); the region renders empty (native webview absent in plain Chrome —
  the accepted PTY-style caveat).

### Frontend — `src/components/Rail.tsx` + `AppShell.tsx`

- Rail keeps the globe action. Add a dot indicator on the globe when a browser
  is open. AppShell polls `ipc.browser.status()` lightly (e.g. on an interval
  while a workspace is active) and passes `browserActive` to the Rail; the dot
  shows when `status.ok === true`, independent of which tab is open.
- The Browser tab remains a center screen, mutually exclusive with the other
  full-page views (existing toggle pattern). No routing change beyond wiring the
  visibility calls into show/hide of the view.

## Data Flow

```
Agent CLI  ── conclave browser open <url> ──▶ router browser.open
                                              runtime::browser::open
                                              main.add_child(webview, offscreen)
Human opens Browser tab ─▶ InAppBrowserView mounts
   ResizeObserver measures region ─▶ ipc.browser.setBounds ─▶ webview.set_position/size
   effect ─▶ ipc.browser.setVisible(true) ─▶ webview.show()
Human switches tab ─▶ view unmounts ─▶ ipc.browser.setVisible(false) ─▶ webview.hide()
   (page stays loaded; agent goto/click/type/snapshot keep working)
Rail polls ipc.browser.status() ─▶ dot when ok
```

## Error Handling

- Missing main window on `open` → `BrowserError::Webview` (hard error; should
  never happen in a running app).
- `set_bounds`/`set_visible` with no browser open → graceful `ok` no-op (mirrors
  `status`/`close` "absent browser reported gracefully").
- All existing `NotOpen`/`InvalidUrl`/`Page`/`Timeout` paths unchanged.

## Testing

- Existing `runtime::browser` pure-helper unit tests (URL normalize, clamp,
  JS escaping, exception-safe IIFEs) are untouched and must stay green.
- New `commands::browser` unit tests: `setBounds` payload parsing (valid + bad
  args), `setVisible` payload parsing.
- Fixture handlers for `setBounds`/`setVisible` added with fixed literals; a
  missing handler must still throw loudly (repo convention).
- **UI Pixel Gate** (`pnpm uishot browser` default + empty, inspect PNGs): the
  native webview does not render in fixture mode, so the region shows empty —
  the accepted caveat, same class as empty PTY panes. The gate verifies the
  toolbar/URL bar chrome renders without error.
- Live embedded-webview behavior (real overlay, background-drive, hide/show)
  requires an app rebuild + manual `conclave browser ...` run — called out, not
  gated by uishot.

## Risks / Open Questions

- **Native webview composites on top of the DOM.** Any React UI that must appear
  above the page region (a toolbar dropdown, a global modal) will be occluded
  while the browser tab is active. Mitigation for V1: the toolbar has no
  overlapping popovers; if one is added later, hide the webview
  (`set_visible(false)`) for its duration. Global app modals are only reachable
  from other tabs, where the webview is already hidden.
- **Bounds sync jitter** on rapid resize — mitigated by debouncing `setBounds`
  to animation frames; a brief lag in the overlay is acceptable.
- **Rounded corners / clipping** of the region are not honored by the native
  overlay (square edges). Accepted for V1.

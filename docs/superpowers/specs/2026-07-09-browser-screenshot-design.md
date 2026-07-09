# In-App Browser: Pixel Screenshot (`browser screenshot`)

**Date:** 2026-07-09
**Status:** Approved design
**Topic:** Add a `browser screenshot` capability so an agent can capture real
pixels of the embedded in-app browser page, closing the gap that forced agents
to fall back to a separate Chrome (Playwright / chrome-devtools MCP) for UI
pixel review. Deferred from the tab-embed work (`docs/superpowers/plans/2026-07-09-inapp-browser-tab-embed.md`).

## Problem

The in-app browser (now a native child webview embedded in the main window's
Browser tab) has no pixel capture — WebKit exposes no CDP surface. An agent
doing a UI pixel review therefore abandons the Conclave browser and launches a
separate Chrome, re-injecting any auth state, and reviews a *different* browser
than the one it drove. We want the agent to capture the page it is already
driving, in-app.

## Goals

- `conclave browser screenshot [output-path] [--width N] [--height N]` captures
  the current embedded page to a PNG and returns the file path.
- Works **headless** — when no human is on the Browser tab (the webview is
  hidden 1×1 offscreen per the embed model), capture still produces a correct
  full-size image, because the review use case runs in the background.
- The returned path is readable by the agent (inside its workspace sandbox).

## Non-Goals

- No UI button — the capability is agent-driven via CLI/IPC; the human already
  sees the live page in the Browser tab.
- macOS only. Other platforms return a clear "unsupported" error. (Conclave's
  primary target is macOS; the capture uses `WKWebView.takeSnapshot`.)
- No screen-region capture, no Screen Recording permission, no CDP/Playwright.
- No multi-format output — PNG only.

## Chosen Approach: `WKWebView.takeSnapshot` via objc2

`Webview::with_webview(|pw| ...)` hands a `PlatformWebview` whose `.inner()`
returns the `*mut c_void` WKWebView pointer on macOS. Cast it to
`objc2_web_kit::WKWebView` and call
`takeSnapshotWithConfiguration:completionHandler:`, which renders the web
content to an `NSImage` regardless of on-screen occlusion. Convert the
`NSImage` to PNG bytes via `NSBitmapImageRep` (`objc2-app-kit`).

`objc2` (0.6), `objc2-web-kit` (0.3.2), and `objc2-app-kit` (0.3.2) are already
in the dependency tree via wry; this adds them as direct macOS-target deps (no
new download).

**Rejected alternatives:** screen-region capture (`xcap`/`screencapture`) needs
Screen Recording permission and captures whatever is on screen — wrong pixels
when the app is occluded or the tab is hidden; DOM serialization (html2canvas)
can't capture canvas/video/cross-origin content and isn't real pixels.

## Architecture

### CLI — `src-tauri/src/bin/conclave-cli.rs` + `commands::cli::map_argv`

Add `browser screenshot [output-path] [--width N] [--height N]`:
- The CLI runs in the agent's cwd. It resolves `output-path` (default
  `./browser-screenshot.png`) to an **absolute** path against the agent's cwd
  *before* sending the router command, so the app process (a different cwd)
  writes the file where the agent expects it. The resolved path lands inside
  the agent's workspace, so the agent can `Read` it under its sandbox.
- Payload: `{ path: <absolute>, width?: N, height?: N }`.
- Add a help row; unit-test the subcommand parse + bad-arg cases (mirrors the
  existing `browser` verbs).

### Backend — `src-tauri/src/engine/runtime/browser.rs`

Add (behind `#[cfg(target_os = "macos")]` for the native path):

```
pub async fn screenshot(
    app: &AppHandle,
    path: &str,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<BrowserShot, BrowserError>
```

`BrowserShot { path: String, width: f64, height: f64 }` (a new camelCase result
struct mirrored in TS).

Flow:
1. `require_webview(app)?` — `NotOpen` when no browser.
2. Resolve capture dims via a pure helper `resolve_capture_size(width, height)`
   → `(w, h)` defaulting to `1280.0 × 800.0`, each clamped to a sane range
   (min 1, max e.g. 10000) — unit-tested.
3. Record the webview's current size (`bounds()`), `set_size` it to the capture
   size, snapshot, then **restore the recorded size in all paths** (success or
   error) so a background capture never disturbs a human-visible layout.
4. Bridge the async `takeSnapshotWithConfiguration:completionHandler:` to async
   with a `oneshot` + timeout (same pattern as `eval_value`; reuse
   `EVAL_TIMEOUT` or a dedicated `SNAPSHOT_TIMEOUT`). The completion handler
   receives an `NSImage` (or error); convert to PNG bytes on the main thread
   via `NSBitmapImageRep` and send the `Vec<u8>` (or an error string) through
   the channel.
5. Write the PNG bytes to `path` (`std::fs::write`), mapping IO errors to
   `BrowserError::Webview`. Return `BrowserShot { path, width, height }`.

Non-macOS build: a `#[cfg(not(target_os = "macos"))]` `screenshot` that returns
`BrowserError::Webview("screenshot is only supported on macOS")`.

New `BrowserError` variant is not required — reuse `NotOpen`/`Webview`/`Timeout`.

### Command / router / IPC

- `commands::browser::screenshot` — parse `{ path, width?, height? }`
  (`AppError::Invalid` on a missing/blank `path`), call the runtime fn, serialize
  `BrowserShot`. Add `"browser.screenshot"` router arm.
- `src/ipc/types.ts`: `BrowserShot { path: string; width: number; height: number }`.
- `src/ipc/commands.ts`: `"browser.screenshot": { req: { path: string; width?: number; height?: number }; res: BrowserShot }` + `ipc.browser.screenshot`.
- `src/ipc/index.ts`: export `BrowserShot`.
- Fixtures (`default.ts`, `empty.ts`): no-op handler returning a fixed literal
  (`{ path: "/tmp/browser-screenshot.png", width: 1280, height: 800 }`).

### tool-map SKILL.md

Add a `browser screenshot` row so agents learn the verb.

## Data Flow

```
Agent: conclave browser screenshot ./review.png --width 1440
  CLI resolves ./review.png → /Users/.../workspace/review.png, sends
  browser.screenshot { path, width:1440 }
    → commands::browser::screenshot → runtime::browser::screenshot
        require_webview → record size → set_size(1440×800)
        with_webview(|wk| takeSnapshot(cfg, |img| tx.send(png_bytes)))
        oneshot ← png bytes (≤ timeout) → restore size → fs::write(path)
    → BrowserShot { path, width:1440, height:800 }
  Agent: Read /Users/.../workspace/review.png  (real pixels)
```

## Error Handling

- No browser open → `NotOpen` (`AppError::Invalid`).
- Blank/empty `path` → `AppError::Invalid` at the command layer.
- takeSnapshot error / no NSImage → `BrowserError::Webview` with the native
  error text; size still restored.
- Snapshot exceeds timeout → `BrowserError::Timeout`; size still restored.
- PNG encode / file write failure → `BrowserError::Webview`.
- Non-macOS → `BrowserError::Webview("screenshot is only supported on macOS")`.

## Testing

- **Pure helper** `resolve_capture_size` — unit tests: defaults (None,None →
  1280×800), explicit values pass through, clamps below-min and above-max.
- **CLI** `map_argv` — unit tests: `browser screenshot` with no path (default),
  with a path, with `--width/--height`, and bad-arg cases.
- **Command parse** — a blank `path` yields `AppError::Invalid` without needing
  a live app (parse-before-app-handle, mirrors `setBounds`/`setVisible`).
- **Native takeSnapshot** — not unit-testable (objc interop needs a live
  WKWebView); verified by an app rebuild + manual `conclave browser open <url>`
  then `conclave browser screenshot`, opening the resulting PNG. Called out as
  a manual gate, same class as the embedded-webview behavior.
- Fixtures make the frontend typecheck/build; no uishot change (no UI surface).

## Risks / Open Questions

- **Layout settle after resize.** `set_size` triggers a reflow; `takeSnapshot`
  fired immediately may capture pre-reflow content. Mitigation: dispatch the
  snapshot on the next main-runloop tick after the resize (or a short fixed
  delay) so WebKit completes a layout pass first. The implementation plan will
  pin the exact mechanism; documented here as the one real correctness risk.
- **Resize side effect.** Temporarily resizing a live webview is observable to
  the page (a `resize` event); acceptable for a capture, and the size is
  restored immediately.
- **objc2 version drift.** The `WKWebView.takeSnapshot` binding lives in
  `objc2-web-kit` 0.3.2, pinned transitively by wry 0.55; adding it as a direct
  dep must match that version to avoid a duplicate objc2 graph.

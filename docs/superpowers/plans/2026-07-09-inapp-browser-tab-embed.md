# In-App Browser Tab Embedding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the Conclave in-app agent browser inside the main window's Browser tab (a native child webview overlaying the center pane) instead of a separate OS window.

**Architecture:** Add a second `Webview` to the main window via `Window::add_child`, positioned over the Browser tab's center-pane rectangle. React reserves an empty region, reports its bounding rect (`browser.setBounds`), and shows/hides the webview on tab switch (`browser.setVisible`) — the page stays loaded in the background so agents keep driving it. Every existing `browser.*` tool keeps working by switching `get_webview_window` → `get_webview`.

**Tech Stack:** Rust + Tauri 2.11.3 (multi-webview via the `unstable` feature), React + TypeScript, Vitest/cargo test.

## Global Constraints

- **Tauri `unstable` feature is REQUIRED** — `Window::add_child` is gated behind `#[cfg(all(desktop, feature = "unstable"))]`. `src-tauri/Cargo.toml` must set `tauri = { version = "2", features = ["unstable"] }`. Without it the backend does not compile.
- **UI copy is English** (app UI language), replies to the user stay Thai.
- **Fixtures use fixed literals only** — no `Date.now()`; a missing fixture handler THROWS by design (never swallow).
- **UI Pixel Gate (standing protocol):** before marking work READY, run `pnpm uishot browser` (default + empty), OPEN and LOOK at each PNG with the Read tool, attach shot paths, and record via `conclave task gate`.
- **Browser label** stays `agent-browser`; one embedded browser per app process (V1, unchanged).
- **No CLI changes** — `setBounds`/`setVisible` are UI-only plumbing; never added to the `conclave` verb map (an agent never repositions the human's viewport).
- **No screenshot/pixel capture** in this plan (separate task).

---

## File Structure

- `src-tauri/Cargo.toml` — enable the `unstable` tauri feature.
- `src-tauri/src/engine/runtime/browser.rs` — MODIFY: embed via `add_child`, `get_webview` lookups, new `resolve_bounds` pure helper + `Bounds` struct, new `set_bounds`/`set_visible`, `state_from` drops title.
- `src-tauri/src/engine/commands/browser.rs` — MODIFY: `open` payload gains optional `bounds`; new `set_bounds`/`set_visible` handlers.
- `src-tauri/src/engine/router.rs` — MODIFY: two new arms.
- `src/ipc/types.ts` — MODIFY: `BrowserBounds`.
- `src/ipc/commands.ts` — MODIFY: `browser.setBounds`/`browser.setVisible`, `open` req gains `bounds?`.
- `src/ipc/index.ts` — MODIFY: export `BrowserBounds`.
- `src/fixtures/scenarios/default.ts`, `src/fixtures/scenarios/empty.ts` — MODIFY: no-op `setBounds`/`setVisible`; drop now-unused `browser.snapshot` from default.
- `src/components/InAppBrowserView.tsx` — MODIFY: remove DOM inspector; add overlay region + bounds sync + visibility.
- `src/components/Rail.tsx` — MODIFY: `browserActive` dot on the globe.
- `src/components/AppShell.tsx` — MODIFY: poll `browser.status`, pass `browserActive` to Rail.

---

### Task 1: Backend — embed as child webview + bounds/visibility

**Files:**
- Modify: `src-tauri/Cargo.toml:25`
- Modify: `src-tauri/src/engine/runtime/browser.rs`
- Test: `src-tauri/src/engine/runtime/browser.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Produces:
  - `pub struct Bounds { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }` (Deserialize, camelCase)
  - `pub async fn open(app: &AppHandle, url: &str, bounds: Option<Bounds>) -> Result<BrowserState, BrowserError>`
  - `pub async fn set_bounds(app: &AppHandle, bounds: Bounds) -> Result<BrowserState, BrowserError>`
  - `pub async fn set_visible(app: &AppHandle, visible: bool) -> Result<BrowserState, BrowserError>`
  - unchanged: `goto/status/snapshot/click/type_text/eval_json/close`
- Consumes: nothing from other tasks.

- [ ] **Step 1: Write the failing test for `resolve_bounds`**

Add to the `#[cfg(test)] mod tests` in `src-tauri/src/engine/runtime/browser.rs`:

```rust
    #[test]
    fn resolve_bounds_uses_given_rect_and_clamps_negative_size() {
        let (pos, size) = resolve_bounds(Some(Bounds { x: 40.0, y: 12.0, width: 800.0, height: -5.0 }));
        assert_eq!(pos, tauri::LogicalPosition::new(40.0, 12.0));
        // A negative height from a mid-layout measurement must clamp to 0, never
        // pass a negative size to the platform.
        assert_eq!(size, tauri::LogicalSize::new(800.0, 0.0));
    }

    #[test]
    fn resolve_bounds_defaults_offscreen_when_absent() {
        // Before React reports a rect, the webview must sit offscreen so it never
        // flashes as a stray overlay over the app chrome.
        let (pos, size) = resolve_bounds(None);
        assert_eq!(pos, tauri::LogicalPosition::new(OFFSCREEN, OFFSCREEN));
        assert_eq!(size, tauri::LogicalSize::new(1.0, 1.0));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib engine::runtime::browser::tests::resolve_bounds 2>&1 | tail -15`
Expected: FAIL to compile — `resolve_bounds`, `Bounds`, `OFFSCREEN` not found.

- [ ] **Step 3: Enable the `unstable` feature**

In `src-tauri/Cargo.toml`, change line 25 from:

```toml
tauri = { version = "2", features = [] }
```

to:

```toml
tauri = { version = "2", features = ["unstable"] }
```

- [ ] **Step 4: Add the `Bounds` struct, `OFFSCREEN`, and `resolve_bounds` helper**

In `src-tauri/src/engine/runtime/browser.rs`, update the Tauri import line (currently `use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};`) to:

```rust
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewBuilder, WebviewUrl};
```

Add near the other constants (after `HARD_MAX_TEXT`):

```rust
/// Offscreen anchor for a browser opened before React has reported its region
/// rectangle — keeps the native overlay from flashing over the app chrome until
/// the first `set_bounds` positions it.
const OFFSCREEN: f64 = -10_000.0;
```

Add the payload struct next to the other result types (after `BrowserState`), plus the pure resolver in the "Pure helpers" section (after `clamp_max_text`):

```rust
/// Viewport rectangle (logical pixels) the Browser tab reserves for the native
/// webview overlay. Mirrored by `BrowserBounds` in `src/ipc/types.ts`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Resolve an optional region rect into a concrete `(position, size)`. Absent →
/// offscreen 1×1 (hidden until React reports a rect); present → the rect with
/// non-negative width/height (a mid-layout measurement can momentarily be
/// negative and must never reach the platform).
fn resolve_bounds(bounds: Option<Bounds>) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    match bounds {
        Some(b) => (
            LogicalPosition::new(b.x, b.y),
            LogicalSize::new(b.width.max(0.0), b.height.max(0.0)),
        ),
        None => (
            LogicalPosition::new(OFFSCREEN, OFFSCREEN),
            LogicalSize::new(1.0, 1.0),
        ),
    }
}
```

- [ ] **Step 5: Run the resolver test to verify it passes**

Run: `cd src-tauri && cargo test --lib engine::runtime::browser::tests::resolve_bounds 2>&1 | tail -8`
Expected: PASS (2 tests). It compiles because `resolve_bounds` is Tauri-type-only, no window needed.

- [ ] **Step 6: Switch the window lookup to a child `Webview` and drop title**

Replace the `window`/`require_window`/`state_from` block:

```rust
/// The single shared browser webview, if it is currently open.
fn webview(app: &AppHandle) -> Option<Webview> {
    app.get_webview(BROWSER_LABEL)
}

fn require_webview(app: &AppHandle) -> Result<Webview, BrowserError> {
    webview(app).ok_or(BrowserError::NotOpen)
}

/// Best-effort current URL for a state reply. `Webview` (a child webview) has no
/// page-title getter — title belongs to the window — so embedded state carries
/// only the URL; the human reads the page title from the live page itself, and
/// agents get it from `snapshot`.
fn state_from(view: &Webview) -> BrowserState {
    BrowserState {
        ok: true,
        url: view.url().ok().map(|u| u.to_string()),
        title: None,
        message: None,
    }
}
```

Then update `eval_value`'s parameter type from `&WebviewWindow` to `&Webview` (only the type name changes; the body is identical).

- [ ] **Step 7: Rewrite `open` to add a child webview; update the other tools**

Replace `open` with:

```rust
/// Open the browser at `url`. If already open, navigate the existing webview and
/// show it. Otherwise add a child webview to the main window at `bounds` (or
/// offscreen+hidden when React has not reported a rect yet).
pub async fn open(
    app: &AppHandle,
    url: &str,
    bounds: Option<Bounds>,
) -> Result<BrowserState, BrowserError> {
    let target = normalize_url(url)?;
    if let Some(view) = webview(app) {
        view.navigate(target).map_err(|e| BrowserError::Webview(e.to_string()))?;
        let _ = view.show();
        return Ok(state_from(&view));
    }
    let window = app
        .get_window("main")
        .ok_or_else(|| BrowserError::Webview("main window not found".into()))?;
    let (position, size) = resolve_bounds(bounds);
    let view = window
        .add_child(
            WebviewBuilder::new(BROWSER_LABEL, WebviewUrl::External(target)),
            position,
            size,
        )
        .map_err(|e| BrowserError::Webview(e.to_string()))?;
    // No rect yet → keep it hidden until React positions and shows it.
    if bounds.is_none() {
        let _ = view.hide();
    }
    Ok(state_from(&view))
}
```

Add the two new tools after `close`:

```rust
/// Position/resize the embedded webview over the Browser tab's reserved region.
/// Graceful no-op `ok` when no browser is open.
pub async fn set_bounds(app: &AppHandle, bounds: Bounds) -> Result<BrowserState, BrowserError> {
    if let Some(view) = webview(app) {
        let (position, size) = resolve_bounds(Some(bounds));
        view.set_position(position).map_err(|e| BrowserError::Webview(e.to_string()))?;
        view.set_size(size).map_err(|e| BrowserError::Webview(e.to_string()))?;
    }
    Ok(BrowserState { ok: true, url: None, title: None, message: None })
}

/// Show/hide the embedded webview on tab switch WITHOUT closing it — the page
/// stays loaded so an agent keeps driving it in the background. Graceful no-op
/// `ok` when no browser is open.
pub async fn set_visible(app: &AppHandle, visible: bool) -> Result<BrowserState, BrowserError> {
    if let Some(view) = webview(app) {
        let res = if visible { view.show() } else { view.hide() };
        res.map_err(|e| BrowserError::Webview(e.to_string()))?;
    }
    Ok(BrowserState { ok: true, url: None, title: None, message: None })
}
```

Update the remaining tools' lookups (bodies otherwise unchanged):
- `goto`: `let win = require_window(app)?;` → `let view = require_webview(app)?;` and `win.navigate` → `view.navigate`, `state_from(&win)` → `state_from(&view)`.
- `status`: `match window(app)` → `match webview(app)`, `Some(win) => Ok(state_from(&win))` → `Some(view) => Ok(state_from(&view))`.
- `snapshot`/`click`/`type_text`/`eval_json`: `let win = require_window(app)?;` → `let view = require_webview(app)?;`, pass `&view` to `eval_value`.
- `close`: `if let Some(win) = window(app)` → `if let Some(view) = webview(app)`, `win.close()` → `view.close()`.

- [ ] **Step 8: Build to verify the Tauri-bound code compiles**

Run: `cd src-tauri && cargo build --lib 2>&1 | tail -20`
Expected: compiles clean (0 errors). Warnings unrelated to this change are acceptable.

- [ ] **Step 9: Run the browser tests**

Run: `cd src-tauri && cargo test --lib engine::runtime::browser 2>&1 | tail -8`
Expected: PASS — all existing pure-helper tests plus the 2 new `resolve_bounds` tests.

- [ ] **Step 10: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/engine/runtime/browser.rs
git commit -m "feat(browser): embed agent browser as a child webview with bounds/visibility

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Backend — command handlers + router arms

**Files:**
- Modify: `src-tauri/src/engine/commands/browser.rs`
- Modify: `src-tauri/src/engine/router.rs:123-130`
- Test: `src-tauri/src/engine/commands/browser.rs` (`#[cfg(test)]` module — new)

**Interfaces:**
- Consumes: `browser::{open, set_bounds, set_visible, Bounds}` from Task 1.
- Produces: router commands `browser.setBounds`, `browser.setVisible`; `browser.open` payload accepts optional `bounds`.

- [ ] **Step 1: Write failing tests for the new payload parsing**

Add a test module at the end of `src-tauri/src/engine/commands/browser.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Malformed payloads must be rejected as `Invalid` BEFORE any app-handle
    // access, so these run without a live Tauri app.
    #[tokio::test]
    async fn set_bounds_rejects_malformed_payload() {
        let state = AppState::for_tests().await;
        let err = set_bounds(&state, json!({ "x": 1.0 })).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn set_visible_rejects_malformed_payload() {
        let state = AppState::for_tests().await;
        let err = set_visible(&state, json!({ "nope": true })).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib engine::commands::browser::tests 2>&1 | tail -15`
Expected: FAIL to compile — `set_bounds`, `set_visible` not found.

- [ ] **Step 3: Add the request structs, handlers, and bounds on `open`**

In `src-tauri/src/engine/commands/browser.rs`, add `Bounds` to the runtime import and a visibility struct:

```rust
use crate::engine::runtime::browser::{self, Bounds, BrowserError};
```

Add after `EvalReq`:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisibleReq {
    visible: bool,
}
```

Change `UrlReq` to carry optional bounds (used only by `open`; `goto` ignores it):

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UrlReq {
    url: String,
    #[serde(default)]
    bounds: Option<Bounds>,
}
```

Update `open` to forward bounds:

```rust
pub async fn open(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<UrlReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(browser::open(app, &req.url, req.bounds).await.map_err(to_app_err)?)
}
```

Add the two new handlers after `close`:

```rust
pub async fn set_bounds(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<Bounds>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(browser::set_bounds(app, req).await.map_err(to_app_err)?)
}

pub async fn set_visible(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<VisibleReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(browser::set_visible(app, req.visible).await.map_err(to_app_err)?)
}
```

- [ ] **Step 4: Wire the router arms**

In `src-tauri/src/engine/router.rs`, after the `"browser.close"` arm (line 130), add:

```rust
        "browser.setBounds" => browser::set_bounds(state, payload).await,
        "browser.setVisible" => browser::set_visible(state, payload).await,
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --lib engine::commands::browser 2>&1 | tail -8`
Expected: PASS (both new tests). If `AppState::for_tests` is unavailable, confirm the helper name with `rg "fn for_tests" src/engine` and use the found name.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/commands/browser.rs src-tauri/src/engine/router.rs
git commit -m "feat(browser): setBounds/setVisible commands + open bounds payload

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Frontend IPC types, commands, and fixtures

**Files:**
- Modify: `src/ipc/types.ts`
- Modify: `src/ipc/commands.ts`
- Modify: `src/ipc/index.ts`
- Modify: `src/fixtures/scenarios/default.ts`, `src/fixtures/scenarios/empty.ts`

**Interfaces:**
- Consumes: backend `browser.setBounds`/`browser.setVisible` from Task 2.
- Produces: `ipc.browser.setBounds(req)`, `ipc.browser.setVisible(req)`, `ipc.browser.open` accepting `{ url; bounds? }`, and the `BrowserBounds` type — all consumed by Task 4/5.

- [ ] **Step 1: Add the `BrowserBounds` type**

In `src/ipc/types.ts`, after the `BrowserSnapshot` interface, add:

```typescript
// Viewport rectangle the Browser tab reserves for the native webview overlay
// (logical pixels). Mirrors the Rust `Bounds` struct.
export interface BrowserBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}
```

- [ ] **Step 2: Extend the `Commands` map and `ipc.browser` namespace**

In `src/ipc/commands.ts`, add `BrowserBounds` to the type import block (alongside `BrowserStatus`), change the `browser.open` entry, and add the two commands:

```typescript
  "browser.open": { req: { url: string; bounds?: BrowserBounds }; res: BrowserStatus };
  "browser.setBounds": { req: BrowserBounds; res: BrowserStatus };
  "browser.setVisible": { req: { visible: boolean }; res: BrowserStatus };
```

In the `ipc.browser` object, add:

```typescript
    setBounds: (req: Commands["browser.setBounds"]["req"]) => call("browser.setBounds", req),
    setVisible: (req: Commands["browser.setVisible"]["req"]) => call("browser.setVisible", req),
```

- [ ] **Step 3: Export `BrowserBounds`**

In `src/ipc/index.ts`, add `BrowserBounds` to the `export type { ... } from "./types"` block (next to `BrowserStatus`).

- [ ] **Step 4: Add fixture handlers; drop the now-unused snapshot**

In `src/fixtures/scenarios/default.ts`, remove the `"browser.snapshot": () => ({ ... })` handler (the embedded view no longer fetches snapshots) and add after `"browser.status"`:

```typescript
  // UI-only overlay plumbing — fixture mode has no native webview, so these are
  // no-ops (fixed, no Tauri). A missing handler would throw by design.
  "browser.setBounds": () => ({ ok: true }),
  "browser.setVisible": () => ({ ok: true }),
```

In `src/fixtures/scenarios/empty.ts`, add after `"browser.status"`:

```typescript
  "browser.setBounds": () => ({ ok: true }),
  "browser.setVisible": () => ({ ok: true }),
```

- [ ] **Step 5: Typecheck**

Run: `pnpm -s exec tsc --noEmit 2>&1 | tail -20`
Expected: no errors. (Task 4 removes the last `snapshot` caller; if tsc flags an unused `browser.snapshot` reference here, it is fixed in Task 4 — for now ensure no *type* errors from the new additions.)

- [ ] **Step 6: Commit**

```bash
git add src/ipc/types.ts src/ipc/commands.ts src/ipc/index.ts src/fixtures/scenarios/default.ts src/fixtures/scenarios/empty.ts
git commit -m "feat(browser): frontend IPC for setBounds/setVisible + open bounds

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Frontend — embed the page in the Browser tab

**Files:**
- Modify: `src/components/InAppBrowserView.tsx`

**Interfaces:**
- Consumes: `ipc.browser.{open,goto,status,setBounds,setVisible,close}` from Task 3.
- Produces: an `InAppBrowserView` that positions/shows the native overlay while mounted and hides it on unmount. Same props (`workspaceId`, `workspaceName?`, `onClose?`).

- [ ] **Step 1: Replace the component body**

Replace the entire contents of `src/components/InAppBrowserView.tsx` with:

```tsx
import { useCallback, useEffect, useRef, useState } from "react";
import { Globe, X, ArrowRight, RotateCcw } from "lucide-react";
import { ipc } from "../ipc";
import type { BrowserStatus } from "../ipc";

// ── In-app browser tab ───────────────────────────────────────────────────────
// The live page runs in a native child webview (runtime::browser) overlaid on
// the empty region below the toolbar. This component owns the URL bar + status
// and keeps the overlay positioned (setBounds) and shown while mounted; on
// unmount (tab switch) it hides the overlay WITHOUT closing it, so an agent
// keeps driving the page in the background. In fixture mode the overlay is
// absent (no Tauri) and the region renders empty — accepted, like PTY panes.

export interface InAppBrowserViewProps {
  workspaceId: string;
  workspaceName?: string;
  onClose?: () => void;
}

export function InAppBrowserView({ workspaceName, onClose }: InAppBrowserViewProps) {
  const [urlInput, setUrlInput] = useState("");
  const [status, setStatus] = useState<BrowserStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true);
  const regionRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef<number | null>(null);

  // Measure the reserved region and push its viewport rect to the native
  // overlay. Debounced to one animation frame so a burst of resize callbacks
  // collapses into a single setBounds.
  const syncBounds = useCallback(() => {
    if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      const el = regionRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      void ipc.browser
        .setBounds({ x: r.left, y: r.top, width: r.width, height: r.height })
        .catch(() => {});
    });
  }, []);

  const loadStatus = useCallback(async () => {
    try {
      const st = await ipc.browser.status();
      if (!mounted.current) return;
      setStatus(st);
      if (st.ok && st.url && !urlInput) setUrlInput(st.url);
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: status failed", err);
      if (mounted.current) setStatus({ ok: false, message: "couldn't read browser status" });
    }
  }, [urlInput]);

  // On mount: show the overlay, position it, read status. On unmount: hide the
  // overlay (never close) so the page keeps running for background agents.
  useEffect(() => {
    mounted.current = true;
    void ipc.browser.setVisible({ visible: true }).catch(() => {});
    syncBounds();
    void loadStatus();

    const el = regionRef.current;
    const ro = el ? new ResizeObserver(() => syncBounds()) : null;
    if (el && ro) ro.observe(el);
    window.addEventListener("resize", syncBounds);

    return () => {
      mounted.current = false;
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      window.removeEventListener("resize", syncBounds);
      ro?.disconnect();
      void ipc.browser.setVisible({ visible: false }).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const doOpen = useCallback(async () => {
    const target = urlInput.trim();
    if (!target || busy) return;
    setBusy(true);
    setError(null);
    try {
      const el = regionRef.current;
      const r = el?.getBoundingClientRect();
      const bounds = r ? { x: r.left, y: r.top, width: r.width, height: r.height } : undefined;
      const st = await ipc.browser.open({ url: target, bounds });
      if (!mounted.current) return;
      setStatus(st);
      await ipc.browser.setVisible({ visible: true }).catch(() => {});
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: open failed", err);
      if (mounted.current) setError("Couldn't open that URL");
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, [urlInput, busy]);

  const doRefresh = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    await loadStatus();
    if (mounted.current) setBusy(false);
  }, [busy, loadStatus]);

  const isOpen = !!status?.ok;

  return (
    <div className="flex-1 min-h-0 flex flex-col bg-surface">
      {/* Header — matches the other center-pane views' 48px title bar. */}
      <div className="h-12 shrink-0 flex items-center gap-2 px-4 border-b border-overlay/[0.06]">
        <Globe className="w-[18px] h-[18px] text-accent shrink-0" />
        <span className="text-[13px] font-semibold text-text-primary">Browser</span>
        {workspaceName && (
          <span className="text-[11px] text-text-tertiary truncate">· {workspaceName}</span>
        )}
        {onClose && (
          <button
            onClick={onClose}
            title="Close"
            className="ml-auto w-7 h-7 grid place-items-center rounded-md text-text-secondary hover:bg-overlay/[0.05]"
          >
            <X className="w-4 h-4" />
          </button>
        )}
      </div>

      {/* Address toolbar. */}
      <div className="shrink-0 flex items-center gap-2 px-4 py-2.5 border-b border-overlay/[0.06]">
        <input
          value={urlInput}
          onChange={(e) => setUrlInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void doOpen();
          }}
          placeholder="Enter a URL (example.com)…"
          spellCheck={false}
          className="flex-1 min-w-0 h-8 px-3 rounded-lg bg-fill-soft ring-hair text-[12.5px] text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 focus:ring-accent font-mono"
        />
        <button
          onClick={() => void doOpen()}
          disabled={busy || !urlInput.trim()}
          className="h-8 px-3 inline-flex items-center gap-1.5 rounded-lg bg-accent text-white text-[12px] font-medium disabled:opacity-40"
          title="Open / navigate"
        >
          <ArrowRight className="w-[14px] h-[14px]" />
          Open
        </button>
        <button
          onClick={() => void doRefresh()}
          disabled={busy}
          className="h-8 w-8 grid place-items-center rounded-lg ring-hair text-text-secondary hover:bg-overlay/[0.04] disabled:opacity-40"
          title="Refresh status"
        >
          <RotateCcw className={`w-[14px] h-[14px]${busy ? " animate-spin" : ""}`} />
        </button>
      </div>

      {/* Status line. */}
      <div className="shrink-0 flex items-center gap-2 px-4 py-2 border-b border-overlay/[0.06] text-[11.5px]">
        <span
          className={`w-1.5 h-1.5 rounded-full shrink-0 ${isOpen ? "bg-success" : "bg-text-tertiary"}`}
        />
        {isOpen ? (
          <span className="text-text-tertiary truncate font-mono">{status?.url}</span>
        ) : (
          <span className="text-text-tertiary">{status?.message || "No browser open"}</span>
        )}
        {error && <span className="ml-auto text-danger">{error}</span>}
      </div>

      {/* Reserved region — the native webview overlay is positioned over this
          rectangle by syncBounds(). It stays empty in the DOM on purpose. */}
      <div ref={regionRef} className="flex-1 min-h-0 relative">
        {!isOpen && (
          <div className="absolute inset-0 grid place-items-center text-center pointer-events-none">
            <div className="max-w-xs space-y-1.5">
              <Globe className="w-8 h-8 mx-auto text-text-tertiary" />
              <div className="text-[12.5px] text-text-secondary">No browser open</div>
              <div className="text-[11px] text-text-tertiary leading-snug">
                Enter a URL above to open a page. Agents drive the same browser with
                <span className="font-mono"> conclave browser</span>.
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Typecheck (confirms the removed snapshot/inspector left no dangling refs)**

Run: `pnpm -s exec tsc --noEmit 2>&1 | tail -20`
Expected: no errors — `BrowserSnapshot`, `InspectorSection`, and the removed lucide icons (`Link2`, `Type`, `MousePointerClick`) are no longer referenced.

- [ ] **Step 3: Build**

Run: `pnpm -s build 2>&1 | tail -15`
Expected: build succeeds.

- [ ] **Step 4: Commit**

```bash
git add src/components/InAppBrowserView.tsx
git commit -m "feat(browser): embed page overlay in Browser tab, drop DOM inspector UI

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Rail dot indicator + AppShell status poll

**Files:**
- Modify: `src/components/Rail.tsx`
- Modify: `src/components/AppShell.tsx`

**Interfaces:**
- Consumes: `ipc.browser.status()` from Task 3.
- Produces: a `browserActive` boolean flowing AppShell → Rail; a dot on the globe when a browser is open, regardless of the active tab.

- [ ] **Step 1: Add the `browserActive` prop + dot to the Rail**

In `src/components/Rail.tsx`, add to `RailProps` (after `browserOpen?: boolean;`):

```typescript
  browserActive?: boolean;
```

Add it to the destructured params (after `browserOpen,`):

```typescript
  browserActive,
```

Replace the Browser `RailActionButton` with a wrapper that carries the dot:

```tsx
          <div className="relative">
            <RailActionButton
              active={!!browserOpen}
              disabled={workspaceScopedDisabled}
              title="Browser view"
              onClick={onOpenBrowser}
            >
              <Globe className="w-[17px] h-[17px]" />
            </RailActionButton>
            {browserActive && !browserOpen && (
              <span
                className="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-success ring-2 ring-fill-soft"
                title="A browser is open"
              />
            )}
          </div>
```

- [ ] **Step 2: Poll status in AppShell and pass the flag**

In `src/components/AppShell.tsx`, add a state hook next to the other browser state (after `const [showBrowser, setShowBrowser] = useState(false);`):

```tsx
  // Whether an agent-driven browser is currently open — polled so the Rail can
  // show a dot even while the human is on another tab.
  const [browserActive, setBrowserActive] = useState(false);
```

Add a polling effect (place it near the other `useEffect`s in the component):

```tsx
  useEffect(() => {
    if (!activeWorkspaceId) {
      setBrowserActive(false);
      return;
    }
    let alive = true;
    const check = () => {
      ipc.browser
        .status()
        .then((st) => {
          if (alive) setBrowserActive(!!st.ok);
        })
        .catch(() => {});
    };
    check();
    const id = window.setInterval(check, 4000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [activeWorkspaceId]);
```

Pass the flag where `<Rail ... browserOpen={showBrowser}` is rendered — add:

```tsx
            browserActive={browserActive}
```

- [ ] **Step 3: Typecheck + build**

Run: `pnpm -s exec tsc --noEmit 2>&1 | tail -15 && pnpm -s build 2>&1 | tail -8`
Expected: no type errors; build succeeds.

- [ ] **Step 4: Commit**

```bash
git add src/components/Rail.tsx src/components/AppShell.tsx
git commit -m "feat(browser): Rail dot indicator + AppShell browser-open poll

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: UI Pixel Gate (standing protocol)

**Files:** none (verification only).

**Interfaces:** consumes the finished `InAppBrowserView` (Task 4) + Rail (Task 5).

- [ ] **Step 1: Render the Browser view (default scenario)**

Run: `pnpm uishot browser`
Expected: exit 0; writes `.shots/browser-default.png`. Grep the console output for `[fixture]` — there must be no unhandled-command errors (all `browser.*` fixtures exist).

- [ ] **Step 2: LOOK at the default PNG**

Use the Read tool on `.shots/browser-default.png`. Confirm: 48px header with globe + "Browser", URL bar + Open + Refresh, status line, and an empty region below (native overlay is absent in fixture mode — expected). No inspector list.

- [ ] **Step 3: Render + LOOK at the empty scenario**

Run: `pnpm uishot browser --scenario empty`
Then Read `.shots/browser-empty.png`. Confirm the "No browser open" empty state renders centered in the region.

- [ ] **Step 4: Record the gate**

Run:
```bash
conclave task gate <ws> inapp-browser-tab-embed -- pnpm uishot browser
```
(substitute the real workspace + task slug). Attach both shot paths in the READY note.

- [ ] **Step 5: Full backend test sweep before READY**

Run: `cd src-tauri && cargo test --lib 2>&1 | tail -5`
Expected: all tests pass (existing suite + the new `resolve_bounds` and command-parse tests).

---

## Self-Review Notes

- **Spec coverage:** embed via child webview (T1), `get_webview` switch (T1), `set_bounds`/`set_visible` + background-drive (T1–T5), remove inspector UI (T4), Rail dot + background load (T5), UI-only (no CLI) plumbing (T2), fixtures + Pixel Gate (T3/T6). Screenshot explicitly out of scope. Covered.
- **Layering risk** (native overlay on top of DOM): the empty-state hint uses `pointer-events-none` and sits under the overlay only when a browser is open (overlay covers it); when closed there's no overlay, so the hint shows. Toolbar has no popovers, matching the spec's V1 mitigation.
- **`unstable` feature** is called out as a hard global constraint and lands in T1 Step 3 before any `add_child` use.
- **Title drop**: `state_from` sets `title: None` because `Webview` exposes no title getter; the status line and `BrowserStatus` consumers already treat `title`/`url` as optional.

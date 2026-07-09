# Browser Screenshot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `conclave browser screenshot [path] [--width N] [--height N]` that captures real pixels of the embedded in-app browser to a PNG file and returns its path.

**Architecture:** On macOS, reach the embedded child webview's native `WKWebView` via `Webview::with_webview().inner()` and call `takeSnapshotWithConfiguration:completionHandler:` (objc2), converting the `NSImage` to PNG. Headless-capable: the webview is temporarily sized to a capture viewport (default 1280×800) and restored afterward. The CLI resolves the output path in the agent's cwd so the app process writes it inside the agent's workspace sandbox.

**Tech Stack:** Rust + Tauri 2.11.3, objc2 0.6 / objc2-web-kit 0.3.2 / objc2-app-kit 0.3.2 / objc2-foundation 0.3 / block2 0.6, React + TypeScript.

## Global Constraints

- **macOS-only native path.** The `screenshot` fn is `#[cfg(target_os = "macos")]`; a `#[cfg(not(target_os = "macos"))]` stub returns `BrowserError::Webview("screenshot is only supported on macOS")`.
- **objc2 versions must match the wry-pinned graph:** `objc2 = "0.6"`, `objc2-web-kit = "0.3.2"`, `objc2-app-kit = "0.3.2"`, `objc2-foundation = "0.3"`, `block2 = "0.6"` — added as `[target.'cfg(target_os = "macos")'.dependencies]` so no duplicate objc2 graph appears.
- **Output is PNG only**, returned as a file path (`BrowserShot { path, width, height }`).
- **Capture default 1280×800**, overridable via `--width`/`--height`, each clamped to `[1, 10000]`.
- **Webview size is always restored** after a capture (success or any error path).
- **No UI button** (agent-driven via CLI/IPC only). UI copy English; fixtures fixed literals.
- Browser label stays `agent-browser`; the `browser.*` family already exists.

---

## File Structure

- `src-tauri/Cargo.toml` — add the five macOS-target objc2/block2 deps.
- `src-tauri/src/engine/runtime/browser.rs` — `BrowserShot` struct, `resolve_capture_size` pure helper, `SNAPSHOT_TIMEOUT`, the `screenshot` fn (macOS native + non-macOS stub).
- `src-tauri/src/engine/commands/browser.rs` — `screenshot` command handler.
- `src-tauri/src/engine/router.rs` — `browser.screenshot` arm.
- `src-tauri/src/bin/conclave-cli.rs` — resolve the output path in the agent's cwd (`expand_self_args`).
- `src-tauri/src/engine/commands/cli.rs` — `browser screenshot` in `map_argv` + help.
- `src/ipc/types.ts`, `src/ipc/commands.ts`, `src/ipc/index.ts` — `BrowserShot` + command.
- `src/fixtures/scenarios/default.ts`, `empty.ts` — no-op fixture handler.
- `src-tauri/skills/tool-map/SKILL.md` — a `browser screenshot` row.

---

### Task 1: Backend deps + `BrowserShot` + `resolve_capture_size`

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/engine/runtime/browser.rs`
- Test: `src-tauri/src/engine/runtime/browser.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub struct BrowserShot { pub path: String, pub width: f64, pub height: f64 }` (Serialize, camelCase)
  - `fn resolve_capture_size(width: Option<f64>, height: Option<f64>) -> (f64, f64)`
  - `const SNAPSHOT_TIMEOUT: Duration`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `src-tauri/src/engine/runtime/browser.rs`:

```rust
    #[test]
    fn resolve_capture_size_defaults_and_clamps() {
        assert_eq!(resolve_capture_size(None, None), (1280.0, 800.0));
        assert_eq!(resolve_capture_size(Some(1440.0), Some(900.0)), (1440.0, 900.0));
        // below-min clamps up to 1, above-max clamps down to 10000
        assert_eq!(resolve_capture_size(Some(0.0), Some(-4.0)), (1.0, 1.0));
        assert_eq!(resolve_capture_size(Some(99999.0), None), (10000.0, 800.0));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib engine::runtime::browser::tests::resolve_capture_size 2>&1 | tail -8`
Expected: FAIL to compile — `resolve_capture_size`, `BrowserShot` not found.

- [ ] **Step 3: Add the macOS deps**

In `src-tauri/Cargo.toml`, after the existing `[dependencies]` block, add a macOS-target section (if a `[target.'cfg(target_os = "macos")'.dependencies]` table already exists, add the keys into it instead):

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-foundation = "0.3"
objc2-web-kit = { version = "0.3.2", features = ["WKWebView", "WKSnapshotConfiguration", "block2"] }
objc2-app-kit = { version = "0.3.2", features = ["NSImage", "NSBitmapImageRep", "NSGraphics"] }
block2 = "0.6"
```

- [ ] **Step 4: Add the struct, constant, and pure helper**

In `src-tauri/src/engine/runtime/browser.rs`, add `use std::time::Duration;` if not already present (it is — `EVAL_TIMEOUT` uses it). Add after the `BrowserSnapshot`/result structs:

```rust
/// Result of `screenshot` — the written PNG's path and the capture dimensions
/// (logical px). Mirrored by `BrowserShot` in `src/ipc/types.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserShot {
    pub path: String,
    pub width: f64,
    pub height: f64,
}
```

Add near the other constants:

```rust
/// Round-trip budget for a `takeSnapshot` capture (renders + PNG encode).
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(15);

/// Default capture viewport when the caller gives no size (logical px).
const DEFAULT_CAPTURE_W: f64 = 1280.0;
const DEFAULT_CAPTURE_H: f64 = 800.0;
/// Hard bounds so a bad `--width/--height` can't ask for a 0-px or absurd canvas.
const MIN_CAPTURE_PX: f64 = 1.0;
const MAX_CAPTURE_PX: f64 = 10_000.0;
```

Add to the "Pure helpers" section:

```rust
/// Resolve the capture viewport: absent → default; present → clamped to
/// `[MIN_CAPTURE_PX, MAX_CAPTURE_PX]`. Never returns a non-positive dimension.
fn resolve_capture_size(width: Option<f64>, height: Option<f64>) -> (f64, f64) {
    let clamp = |v: f64| v.clamp(MIN_CAPTURE_PX, MAX_CAPTURE_PX);
    (
        width.map(clamp).unwrap_or(DEFAULT_CAPTURE_W),
        height.map(clamp).unwrap_or(DEFAULT_CAPTURE_H),
    )
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib engine::runtime::browser::tests::resolve_capture_size 2>&1 | tail -8`
Expected: PASS.

- [ ] **Step 6: Confirm the crate still builds with the new deps**

Run: `cd src-tauri && cargo build --lib 2>&1 | tail -8`
Expected: compiles (downloads/links objc2 crates already present in the graph). 0 errors.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/engine/runtime/browser.rs
git commit -m "feat(browser): add BrowserShot + capture-size helper + macOS objc2 deps

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Native `screenshot` fn (WKWebView.takeSnapshot)

**Files:**
- Modify: `src-tauri/src/engine/runtime/browser.rs`

**Interfaces:**
- Consumes: `BrowserShot`, `resolve_capture_size`, `SNAPSHOT_TIMEOUT`, `require_webview`, `BrowserError` (Task 1 + existing).
- Produces: `pub async fn screenshot(app: &AppHandle, path: &str, width: Option<f64>, height: Option<f64>) -> Result<BrowserShot, BrowserError>`

**Note to implementer:** this is native objc2 interop; it cannot be unit-tested (needs a live WKWebView). The objc2 method names below are verified against the installed 0.3.2 bindings (`takeSnapshotWithConfiguration_completionHandler`, `WKSnapshotConfiguration::new(mtm)` / `setSnapshotWidth`, `NSBitmapImageRep`, `NSBitmapImageFileType::PNG`), but the block-closure pointer types and NSImage→PNG glue may need adjustment to satisfy the compiler — iterate `cargo build --lib` until it compiles clean. Keep the size-save/restore and the oneshot+timeout structure exactly as shown.

- [ ] **Step 1: Add the imports (macOS-gated) and the `screenshot` fn**

In `src-tauri/src/engine/runtime/browser.rs`, add the native fn (place it after `close`). The reference implementation:

```rust
/// Capture the embedded page to a PNG at `path` (absolute; the CLI resolves it
/// in the agent's cwd). Sizes the webview to the capture viewport, snapshots via
/// WKWebView.takeSnapshot, restores the prior size, and writes the PNG. Works
/// headless (the webview may be hidden/offscreen). macOS only.
#[cfg(target_os = "macos")]
pub async fn screenshot(
    app: &AppHandle,
    path: &str,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<BrowserShot, BrowserError> {
    use std::sync::Mutex;

    let view = require_webview(app)?;
    let (w, h) = resolve_capture_size(width, height);

    // Remember the current size so a background capture never disturbs a
    // human-visible layout; restore it in every exit path below.
    let prev = view.bounds().ok();

    view.set_size(LogicalSize::new(w, h))
        .map_err(|e| BrowserError::Webview(e.to_string()))?;
    // Let WebKit run a layout pass at the new size before snapshotting; firing
    // takeSnapshot in the same tick can capture pre-reflow content.
    tokio::time::sleep(Duration::from_millis(150)).await;

    let (tx, rx) = oneshot::channel::<Result<Vec<u8>, String>>();
    let slot = Mutex::new(Some(tx));
    let with_res = view.with_webview(move |platform| {
        use block2::RcBlock;
        use objc2::rc::Retained;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
        use objc2_foundation::{MainThreadMarker, NSDictionary, NSNumber};
        use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

        // with_webview runs on the main thread; the WKWebView pointer comes from
        // the platform handle's inner().
        let send = |slot: &Mutex<Option<oneshot::Sender<Result<Vec<u8>, String>>>>,
                    v: Result<Vec<u8>, String>| {
            if let Ok(mut g) = slot.lock() {
                if let Some(tx) = g.take() {
                    let _ = tx.send(v);
                }
            }
        };
        unsafe {
            let mtm = MainThreadMarker::new_unchecked();
            let wk_ptr = platform.inner() as *mut WKWebView;
            if wk_ptr.is_null() {
                send(&slot, Err("null WKWebView pointer".into()));
                return;
            }
            let wk: &WKWebView = &*wk_ptr;
            let cfg = WKSnapshotConfiguration::new(mtm);
            cfg.setSnapshotWidth(Some(&NSNumber::new_f64(w)));

            let handler = RcBlock::new(
                move |image: *mut NSImage, error: *mut objc2_foundation::NSError| {
                    if !error.is_null() {
                        let msg = (*error).localizedDescription().to_string();
                        send(&slot, Err(format!("takeSnapshot: {msg}")));
                        return;
                    }
                    if image.is_null() {
                        send(&slot, Err("takeSnapshot returned no image".into()));
                        return;
                    }
                    let image: &NSImage = &*image;
                    // NSImage → TIFF → NSBitmapImageRep → PNG NSData → bytes.
                    let Some(tiff) = image.TIFFRepresentation() else {
                        send(&slot, Err("no TIFF representation".into()));
                        return;
                    };
                    let Some(rep) = NSBitmapImageRep::initWithData(
                        NSBitmapImageRep::alloc(),
                        &tiff,
                    ) else {
                        send(&slot, Err("no bitmap rep".into()));
                        return;
                    };
                    let props = NSDictionary::new();
                    let Some(png) =
                        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props)
                    else {
                        send(&slot, Err("PNG encode failed".into()));
                        return;
                    };
                    send(&slot, Ok(png.to_vec()));
                    let _ = &Retained::retain; // keep import used if optimized
                },
            );
            wk.takeSnapshotWithConfiguration_completionHandler(Some(&cfg), &handler);
        }
    });

    // Restore size regardless of how the snapshot went.
    let restore = |view: &Webview, prev: &Option<tauri::dpi::Rect>| {
        if let Some(rect) = prev {
            let _ = view.set_size(rect.size);
        }
    };

    if let Err(e) = with_res {
        restore(&view, &prev);
        return Err(BrowserError::Webview(e.to_string()));
    }

    let captured = tokio::time::timeout(SNAPSHOT_TIMEOUT, rx).await;
    restore(&view, &prev);

    let bytes = captured
        .map_err(|_| BrowserError::Timeout)?
        .map_err(|_| BrowserError::Webview("snapshot callback dropped".into()))?
        .map_err(BrowserError::Page)?;

    std::fs::write(path, &bytes)
        .map_err(|e| BrowserError::Webview(format!("write {path}: {e}")))?;

    Ok(BrowserShot { path: path.to_owned(), width: w, height: h })
}

/// Non-macOS: capture uses a macOS-only native API.
#[cfg(not(target_os = "macos"))]
pub async fn screenshot(
    _app: &AppHandle,
    _path: &str,
    _width: Option<f64>,
    _height: Option<f64>,
) -> Result<BrowserShot, BrowserError> {
    Err(BrowserError::Webview(
        "screenshot is only supported on macOS".into(),
    ))
}
```

If `tauri::dpi::Rect` / `rect.size` don't match the type returned by `Webview::bounds()`, inspect the return type (`cargo doc`-free: it's `tauri_runtime::dpi::Rect { position, size }`) and restore with the concrete `Size` it exposes; the intent is "set_size back to the pre-capture size."

- [ ] **Step 2: Build until it compiles**

Run: `cd src-tauri && cargo build --lib 2>&1 | tail -30`
Expected: compiles clean. Fix objc2 type/method mismatches by consulting the installed bindings under `~/.cargo/registry/src/index.crates.io-*/objc2-web-kit-0.3.2/` and `objc2-app-kit-0.3.2/`. Do NOT change the save/restore or oneshot/timeout logic; only adjust interop glue.

- [ ] **Step 3: Run the browser tests (pure helpers unaffected)**

Run: `cd src-tauri && cargo test --lib engine::runtime::browser 2>&1 | tail -6`
Expected: PASS (existing + `resolve_capture_size`).

- [ ] **Step 4: Clippy the changed crate**

Run: `cd src-tauri && cargo clippy --lib 2>&1 | rg -A4 "browser.rs" | head -30`
Expected: no NEW warnings originating in the `screenshot` fn (pre-existing crate warnings unrelated to this change are acceptable).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/engine/runtime/browser.rs
git commit -m "feat(browser): native WKWebView.takeSnapshot screenshot capture

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Command handler + router arm

**Files:**
- Modify: `src-tauri/src/engine/commands/browser.rs`
- Modify: `src-tauri/src/engine/router.rs`
- Test: `src-tauri/src/engine/commands/browser.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `browser::screenshot`, `browser::BrowserShot` (Task 1/2).
- Produces: router command `browser.screenshot`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `src-tauri/src/engine/commands/browser.rs`:

```rust
    #[tokio::test]
    async fn screenshot_rejects_blank_path() {
        let state = AppState::for_tests().await;
        let err = screenshot(&state, json!({ "path": "" })).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn screenshot_rejects_missing_path() {
        let state = AppState::for_tests().await;
        let err = screenshot(&state, json!({ "width": 800 })).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib engine::commands::browser::tests::screenshot 2>&1 | tail -12`
Expected: FAIL to compile — `screenshot` not found.

- [ ] **Step 3: Add the request struct + handler**

In `src-tauri/src/engine/commands/browser.rs`, add after `EvalReq`:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotReq {
    path: String,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
}
```

Add the handler after `close`:

```rust
pub async fn screenshot(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<ScreenshotReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    if req.path.trim().is_empty() {
        return Err(AppError::Invalid("browser screenshot: path is required".into()));
    }
    let app = app_handle(state)?;
    to_value(
        browser::screenshot(app, &req.path, req.width, req.height)
            .await
            .map_err(to_app_err)?,
    )
}
```

- [ ] **Step 4: Wire the router arm**

In `src-tauri/src/engine/router.rs`, after the `"browser.setVisible"` arm, add:

```rust
        "browser.screenshot" => browser::screenshot(state, payload).await,
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib engine::commands::browser 2>&1 | tail -6`
Expected: PASS (both new tests + existing).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/commands/browser.rs src-tauri/src/engine/router.rs
git commit -m "feat(browser): screenshot command handler + router arm

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: CLI verb + agent-cwd path resolution

**Files:**
- Modify: `src-tauri/src/bin/conclave-cli.rs`
- Modify: `src-tauri/src/engine/commands/cli.rs`
- Test: `src-tauri/src/engine/commands/cli.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: router command `browser.screenshot { path, width?, height? }` (Task 3).
- Produces: `conclave browser screenshot [path] [--width N] [--height N]` → `browser.screenshot`.

**Design note:** the CLI binary runs in the agent's cwd; the app process (server) runs elsewhere. `expand_self_args` (client-side, in `conclave-cli.rs`) resolves the output path to absolute against the agent's cwd BEFORE the argv is sent, so `map_argv` (server-side) receives an already-absolute path and just packages it.

- [ ] **Step 1: Write the failing test for `map_argv`**

Add to the `#[cfg(test)] mod tests` in `src-tauri/src/engine/commands/cli.rs` (near the existing `browser` tests around line 2787):

```rust
    #[test]
    fn browser_screenshot_maps_with_defaults_and_flags() {
        // path is positional; already absolute by the time map_argv sees it.
        assert_eq!(
            ok_method(&["browser", "screenshot", "/ws/shot.png"]),
            "browser.screenshot"
        );
        assert_eq!(
            ok_params(&["browser", "screenshot", "/ws/shot.png"]),
            json!({ "path": "/ws/shot.png" })
        );
        assert_eq!(
            ok_params(&["browser", "screenshot", "/ws/shot.png", "--width", "1440", "--height", "900"]),
            json!({ "path": "/ws/shot.png", "width": 1440.0, "height": 900.0 })
        );
    }

    #[test]
    fn browser_screenshot_rejects_bad_dimension() {
        assert!(map_argv(&to_vec(&["browser", "screenshot", "/ws/s.png", "--width", "abc"])).is_err());
    }
```

If the existing browser tests use a different helper than `ok_method`/`ok_params`/`to_vec`, mirror whatever they use (check lines ~2787-2794).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib engine::commands::cli::tests::browser_screenshot 2>&1 | tail -12`
Expected: FAIL — `map_argv` returns the unknown-subcommand error for `screenshot`.

- [ ] **Step 3: Add the `screenshot` arm to `map_argv`**

In `src-tauri/src/engine/commands/cli.rs`, inside the `"browser" => match ...` block, add an arm before the catch-all `_ =>` (after `Some("close")`):

```rust
            Some("screenshot") => {
                // path is positional (already absolute — the CLI resolved it in
                // the agent's cwd); --width/--height are optional f64 flags.
                let after = argv.get(2..).unwrap_or(&[]);
                let (width_raw, after) = take_flag(after, "--width");
                let (height_raw, after) = take_flag(after, "--height");
                let path = after.first().cloned().ok_or_else(|| {
                    AppError::Invalid("cli: browser screenshot <path> [--width N] [--height N]".into())
                })?;
                if after.len() != 1 {
                    return Err(AppError::Invalid(
                        "cli: browser screenshot <path> [--width N] [--height N]".into(),
                    ));
                }
                let mut params = json!({ "path": path });
                if let Some(raw) = width_raw {
                    let n = raw.parse::<f64>().map_err(|_| {
                        AppError::Invalid("cli: browser screenshot: --width expects a number".into())
                    })?;
                    params["width"] = json!(n);
                }
                if let Some(raw) = height_raw {
                    let n = raw.parse::<f64>().map_err(|_| {
                        AppError::Invalid("cli: browser screenshot: --height expects a number".into())
                    })?;
                    params["height"] = json!(n);
                }
                Ok(("browser.screenshot", params))
            }
```

Update the catch-all error string to include `screenshot`:

```rust
            _ => Err(AppError::Invalid(
                "cli: browser <open|goto|status|snapshot|screenshot|click|type|eval|close> — unknown browser subcommand".into(),
            )),
```

**Verify `take_flag` returns `(Option<String>, Vec<String>)`** (the `snapshot` arm uses it the same way); if its signature differs, adapt the two calls to match.

- [ ] **Step 4: Resolve the output path in the CLI binary**

In `src-tauri/src/bin/conclave-cli.rs`, in `expand_self_args`, add a branch so `browser screenshot` gets its path (positional, default `browser-screenshot.png`) resolved to absolute against the agent's cwd. Add to the `match argv.first().map(String::as_str)` block:

```rust
        Some("browser") if argv.get(1).map(String::as_str) == Some("screenshot") => {
            // Resolve the output path (default ./browser-screenshot.png) to an
            // absolute path in the AGENT's cwd, so the app process — which has a
            // different cwd — writes the PNG where the agent can read it.
            let mut out = argv.clone();
            // The path is the first non-flag token after "screenshot"; flags are
            // --width/--height with a following value. Find it, or inject default.
            let flags = ["--width", "--height"];
            let mut i = 2;
            let mut path_idx: Option<usize> = None;
            while i < out.len() {
                if flags.contains(&out[i].as_str()) {
                    i += 2; // skip flag + its value
                    continue;
                }
                path_idx = Some(i);
                break;
            }
            let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
            match path_idx {
                Some(idx) => {
                    let p = std::path::Path::new(&out[idx]);
                    if p.is_relative() {
                        out[idx] = cwd.join(p).to_string_lossy().into_owned();
                    }
                }
                None => {
                    out.push(cwd.join("browser-screenshot.png").to_string_lossy().into_owned());
                }
            }
            Ok(out)
        }
```

Confirm the surrounding `match` returns `Result<Vec<String>, String>` (it does — other arms return `Ok(out)` / `Err(String)`); if the default-injection ordering matters (flags after the appended path), note that `map_argv`'s `take_flag` strips flags regardless of position, so appending the path at the end is safe.

- [ ] **Step 5: Add the help row**

Find the browser help text in `conclave-cli.rs` (grep `browser open`), and add a line documenting `browser screenshot <path> [--width N] [--height N]` in the same style as the sibling rows.

- [ ] **Step 6: Run tests + build the CLI binary**

Run: `cd src-tauri && cargo test --lib engine::commands::cli::tests::browser 2>&1 | tail -8 && cargo build --bin conclave 2>&1 | tail -5`
Expected: browser map tests PASS; `conclave` binary builds.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/bin/conclave-cli.rs src-tauri/src/engine/commands/cli.rs
git commit -m "feat(browser): CLI screenshot verb + agent-cwd path resolution

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Frontend IPC + fixtures + tool-map

**Files:**
- Modify: `src/ipc/types.ts`, `src/ipc/commands.ts`, `src/ipc/index.ts`
- Modify: `src/fixtures/scenarios/default.ts`, `src/fixtures/scenarios/empty.ts`
- Modify: `src-tauri/skills/tool-map/SKILL.md`

**Interfaces:**
- Consumes: backend `browser.screenshot { path, width?, height? }` → `BrowserShot` (Task 3).
- Produces: `ipc.browser.screenshot(req)` + `BrowserShot` TS type.

- [ ] **Step 1: Add the `BrowserShot` type**

In `src/ipc/types.ts`, after `BrowserBounds`, add:

```typescript
// Result of browser.screenshot — the written PNG path + capture dims. Mirrors
// the Rust `BrowserShot` struct.
export interface BrowserShot {
  path: string;
  width: number;
  height: number;
}
```

- [ ] **Step 2: Extend the Commands map + ipc namespace**

In `src/ipc/commands.ts`, add `BrowserShot` to the type-import block (next to `BrowserBounds`), add the command entry:

```typescript
  "browser.screenshot": { req: { path: string; width?: number; height?: number }; res: BrowserShot };
```

and the method in the `ipc.browser` object:

```typescript
    screenshot: (req: Commands["browser.screenshot"]["req"]) => call("browser.screenshot", req),
```

- [ ] **Step 3: Export `BrowserShot`**

In `src/ipc/index.ts`, add `BrowserShot` to the `export type { ... } from "./types"` block (next to `BrowserBounds`).

- [ ] **Step 4: Add fixture handlers**

In BOTH `src/fixtures/scenarios/default.ts` and `src/fixtures/scenarios/empty.ts`, add after the `browser.setVisible` handler:

```typescript
  "browser.screenshot": () => ({ path: "/tmp/browser-screenshot.png", width: 1280, height: 800 }),
```

- [ ] **Step 5: Add the tool-map row**

In `src-tauri/skills/tool-map/SKILL.md`, find the `browser` verb rows and add one for screenshot, matching the existing table/list format, e.g.:

```
| `browser screenshot <path> [--width N] [--height N]` | Capture the current page to a PNG (macOS). Returns the file path — Read it to see the pixels. |
```

(Match the actual column layout of the surrounding rows.)

- [ ] **Step 6: Typecheck + build**

Run: `cd /Users/detoro/code/codeup && pnpm -s exec tsc --noEmit 2>&1 | tail -15 && pnpm -s build 2>&1 | tail -6`
Expected: 0 type errors; build succeeds.

- [ ] **Step 7: Commit**

```bash
git add src/ipc/types.ts src/ipc/commands.ts src/ipc/index.ts src/fixtures/scenarios/default.ts src/fixtures/scenarios/empty.ts src-tauri/skills/tool-map/SKILL.md
git commit -m "feat(browser): frontend IPC + fixtures + tool-map for screenshot

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Full verification sweep + manual native gate

**Files:** none (verification only).

- [ ] **Step 1: Full backend test sweep**

Run: `cd src-tauri && cargo test --lib 2>&1 | tail -5`
Expected: all pass (existing suite + `resolve_capture_size` + 2 command-parse + 2 CLI-map tests).

- [ ] **Step 2: fmt + clippy**

Run: `cd src-tauri && cargo fmt --check 2>&1 | tail -3 && cargo clippy --lib 2>&1 | tail -5`
Expected: fmt clean; no new clippy warnings from this feature.

- [ ] **Step 3: Frontend typecheck + build**

Run: `cd /Users/detoro/code/codeup && pnpm -s exec tsc --noEmit 2>&1 | tail -5 && pnpm -s build 2>&1 | tail -5`
Expected: 0 errors; build succeeds.

- [ ] **Step 4: Document the manual native gate**

The `takeSnapshot` path can only be verified in a real app (needs a live WKWebView + display). Record these steps in the READY note for the human to run — they are NOT automatable in this environment:

```
1. pnpm tauri build   (or tauri dev)  → launch the app
2. In a Conclave agent's terminal (or a shell with conclave on PATH):
     conclave browser open example.com
     conclave browser screenshot ./shot.png
3. Confirm the command prints a path and ./shot.png is a valid PNG of the page
   (open it). Try --width 1440 --height 900 and confirm the image dimensions.
4. Switch away from the Browser tab, run screenshot again → confirm it still
   captures (headless) and the visible layout is unchanged afterward.
```

- [ ] **Step 5: Report READY** with the automated results (Steps 1-3 output) and the manual gate steps (Step 4) attached.

---

## Self-Review Notes

- **Spec coverage:** CLI verb + path resolution (T4), macOS `takeSnapshot` native + non-macOS stub (T2), `resolve_capture_size` default/clamp + `BrowserShot` (T1), command/router (T3), IPC/types/fixtures/tool-map (T5), size save/restore + oneshot/timeout + layout-settle delay (T2), manual gate (T6). No UI button (correctly absent). Covered.
- **Type consistency:** `screenshot(app, path: &str, width: Option<f64>, height: Option<f64>) -> Result<BrowserShot, _>` is identical across runtime (T2), command (`ScreenshotReq{path, width?, height?}`, T3), router (T3), CLI params `{path, width?, height?}` (T4), and TS `{path; width?; height?}` → `BrowserShot{path,width,height}` (T5). Capture default 1280×800 stated in T1 and matches the CLI/native path.
- **Native-code caveat:** T2's objc2 block is a verified-names reference that the implementer must compile-iterate; the plan pins the load-bearing structure (save/restore, timeout, 150ms settle) and marks only the interop glue as adjustable. This is the one task warranting the most capable model.
- **Layout-settle risk** (spec's open question) is pinned to a 150ms post-resize `sleep` before `takeSnapshot` in T2.

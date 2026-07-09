//! In-app agent browser: ONE shared Tauri `WebviewWindow` (stable label
//! [`BROWSER_LABEL`]) that an agent drives through explicit page tools instead
//! of spinning up Playwright/Puppeteer. `commands::browser` is the only caller;
//! it parses payloads and serializes what these free functions return.
//!
//! # Why a deep module
//!
//! WebView creation, the async `eval_with_callback` → oneshot → timeout dance,
//! selector/string escaping, and the DOM-snapshot JS are all fragile and must
//! stay local. Everything that talks to Tauri lives behind the eight `pub async
//! fn`s below; the pure helpers (URL normalization, `clamp_max_text`, the JS
//! builders) carry no Tauri dependency and are unit-tested without a WebView.
//!
//! # eval semantics (load-bearing)
//!
//! `WebviewWindow::eval_with_callback` serializes the evaluated expression's
//! result to a JSON string and hands it to the callback. Per Tauri's own note
//! the callback is invoked with the JSON of the result, but a *thrown*
//! exception is swallowed on some platforms — so every injected script is an
//! IIFE wrapped in `try/catch` that RETURNS `{ __error: string }` rather than
//! throwing. A page tool that sees `__error` maps it to [`BrowserError::Page`].

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewBuilder, WebviewUrl};
use tokio::sync::oneshot;

/// Stable label for the single shared browser window (plan §Runtime Notes).
/// V1 is one browser per Conclave app process.
const BROWSER_LABEL: &str = "agent-browser";

/// Round-trip budget for an `eval_with_callback` page tool. A remote page can
/// hang or block automation; we never wait forever — [`BrowserError::Timeout`]
/// surfaces a clear failure instead.
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Snapshot body-text cap (plan §Runtime Notes): default and hard maximum. The
/// cap keeps a large page from flooding an agent's context window.
const DEFAULT_MAX_TEXT: usize = 12_000;
const HARD_MAX_TEXT: usize = 50_000;

/// Offscreen anchor for a browser opened before React has reported its region
/// rectangle — keeps the native overlay from flashing over the app chrome until
/// the first `set_bounds` positions it.
const OFFSCREEN: f64 = -10_000.0;

/// Round-trip budget for a `takeSnapshot` capture (renders + PNG encode).
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(15);

/// Default capture viewport when the caller gives no size (logical px).
const DEFAULT_CAPTURE_W: f64 = 1280.0;
const DEFAULT_CAPTURE_H: f64 = 800.0;
/// Hard bounds so a bad `--width/--height` can't ask for a 0-px or absurd canvas.
const MIN_CAPTURE_PX: f64 = 1.0;
const MAX_CAPTURE_PX: f64 = 10_000.0;

// ── Result types (mirrored 1:1 by src/ipc/types.ts, camelCase) ──────────────

/// Result of `open`/`goto`/`status`/`close`. `ok` is false with a `message`
/// when there is no browser to report on (status/close before open) — an
/// absent browser is reported gracefully, not as a hard error.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

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

/// Result of an in-page action (`click`/`type`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Agent-facing DOM/text snapshot (NOT pixels — see plan non-goals).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSnapshot {
    pub url: String,
    pub title: String,
    pub text: String,
    pub headings: Vec<String>,
    pub links: Vec<SnapshotLink>,
    pub inputs: Vec<SnapshotInput>,
    pub buttons: Vec<SnapshotButton>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotLink {
    pub text: String,
    pub href: String,
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInput {
    pub selector: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub input_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotButton {
    pub text: String,
    pub selector: String,
}

/// Result of `screenshot` — the written PNG's path and the capture dimensions
/// (logical px). Mirrored by `BrowserShot` in `src/ipc/types.ts`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserShot {
    pub path: String,
    pub width: f64,
    pub height: f64,
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Runtime-layer failure modes. Manual `Display` impl per this crate's
/// convention (`thiserror` is used at the `AppError`/command layer instead —
/// see `runtime::design_host::DesignHostError`).
#[derive(Debug)]
pub enum BrowserError {
    /// The requested URL was empty, unparseable, or used an unsupported scheme.
    InvalidUrl(String),
    /// A tool that needs an open browser was called with none open.
    NotOpen,
    /// A Tauri/WebView call failed (build, navigate, eval dispatch, …).
    Webview(String),
    /// The injected script returned `{ __error }` — an in-page failure.
    Page(String),
    /// `eval_with_callback` did not answer within [`EVAL_TIMEOUT`].
    Timeout,
}

impl std::fmt::Display for BrowserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrowserError::InvalidUrl(u) => write!(f, "invalid url: {u}"),
            BrowserError::NotOpen => write!(f, "no browser is open — run `browser open <url>` first"),
            BrowserError::Webview(m) => write!(f, "browser webview error: {m}"),
            BrowserError::Page(m) => write!(f, "browser page error: {m}"),
            BrowserError::Timeout => write!(f, "browser page tool timed out"),
        }
    }
}

impl std::error::Error for BrowserError {}

// ── Pure helpers (no Tauri; unit-tested) ────────────────────────────────────

/// Normalize a user-supplied URL string. A missing scheme is filled with
/// `https://`; `about:blank`, `http://`, `https://`, and `file://` are left
/// as-is. An unparseable result fails loudly rather than silently loading a
/// bogus page.
fn normalize_url(input: &str) -> Result<Url, BrowserError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(BrowserError::InvalidUrl("(empty)".into()));
    }
    let has_scheme = trimmed == "about:blank"
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("file://");
    let candidate = if has_scheme {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    Url::parse(&candidate).map_err(|_| BrowserError::InvalidUrl(input.to_owned()))
}

/// Resolve the requested snapshot text cap into a concrete byte budget: absent
/// or non-positive → [`DEFAULT_MAX_TEXT`]; otherwise clamped to
/// [`HARD_MAX_TEXT`].
fn clamp_max_text(requested: Option<i64>) -> usize {
    match requested {
        Some(n) if n > 0 => (n as usize).min(HARD_MAX_TEXT),
        _ => DEFAULT_MAX_TEXT,
    }
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

/// Resolve the capture viewport: absent → default; present → clamped to
/// `[MIN_CAPTURE_PX, MAX_CAPTURE_PX]`. Never returns a non-positive dimension.
fn resolve_capture_size(width: Option<f64>, height: Option<f64>) -> (f64, f64) {
    let clamp = |v: f64| v.clamp(MIN_CAPTURE_PX, MAX_CAPTURE_PX);
    (
        width.map(clamp).unwrap_or(DEFAULT_CAPTURE_W),
        height.map(clamp).unwrap_or(DEFAULT_CAPTURE_H),
    )
}

/// Embed an arbitrary Rust string as a safe JS string literal. `serde_json`
/// emits a valid double-quoted JS string (escaping quotes, backslashes,
/// newlines, control chars), which is exactly a JSON-string-is-a-JS-string
/// guarantee — so a selector or text value can never break out of the script.
fn js_literal(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

/// The DOM/text snapshot script: a self-contained IIFE returning the snapshot
/// object, or `{ __error }` on any in-page failure. `__MAX_TEXT__` is replaced
/// with the resolved cap. `cssPath` emits selectors reusable by `click`/`type`.
fn snapshot_js(max_text: usize) -> String {
    SNAPSHOT_JS_TEMPLATE.replace("__MAX_TEXT__", &max_text.to_string())
}

const SNAPSHOT_JS_TEMPLATE: &str = r##"(function () {
  try {
    function cssPath(el) {
      if (!el || el.nodeType !== 1) return "";
      if (el.id) return "#" + CSS.escape(el.id);
      var parts = [];
      var cur = el;
      while (cur && cur.nodeType === 1 && cur.tagName.toLowerCase() !== "html") {
        var tag = cur.tagName.toLowerCase();
        var parent = cur.parentElement;
        if (!parent) { parts.unshift(tag); break; }
        var sibs = Array.prototype.filter.call(
          parent.children,
          function (c) { return c.tagName === cur.tagName; }
        );
        if (sibs.length > 1) {
          var idx = Array.prototype.indexOf.call(sibs, cur) + 1;
          tag += ":nth-of-type(" + idx + ")";
        }
        parts.unshift(tag);
        if (parent.id) { parts.unshift("#" + CSS.escape(parent.id)); break; }
        cur = parent;
      }
      return parts.join(" > ");
    }
    function clean(s) { return (s || "").replace(/\s+/g, " ").trim(); }
    var MAX = __MAX_TEXT__;
    var bodyText = clean(document.body ? document.body.innerText : "");
    if (bodyText.length > MAX) bodyText = bodyText.slice(0, MAX);
    var headings = Array.prototype.map.call(
      document.querySelectorAll("h1,h2,h3,h4,h5,h6"),
      function (h) { return clean(h.textContent); }
    ).filter(Boolean).slice(0, 100);
    var links = Array.prototype.map.call(
      document.querySelectorAll("a[href]"),
      function (a) { return { text: clean(a.textContent), href: a.href, selector: cssPath(a) }; }
    ).filter(function (l) { return l.href; }).slice(0, 200);
    var inputs = Array.prototype.map.call(
      document.querySelectorAll("input,textarea,select"),
      function (el) {
        var o = { selector: cssPath(el) };
        if (el.type) o.type = el.type;
        if (el.name) o.name = el.name;
        return o;
      }
    ).slice(0, 200);
    var buttons = Array.prototype.map.call(
      document.querySelectorAll("button,[role=button],input[type=submit],input[type=button]"),
      function (b) {
        return { text: clean(b.textContent || b.value), selector: cssPath(b) };
      }
    ).slice(0, 200);
    return {
      url: location.href,
      title: document.title || "",
      text: bodyText,
      headings: headings,
      links: links,
      inputs: inputs,
      buttons: buttons
    };
  } catch (e) {
    return { __error: String(e && e.message ? e.message : e) };
  }
})()"##;

/// Click script for `selector`: `{ ok:true, url }` on a hit, or `{ ok:false,
/// message }` when the selector matches nothing / the click throws.
fn click_js(selector: &str) -> String {
    format!(
        r#"(function () {{
  try {{
    var el = document.querySelector({sel});
    if (!el) return {{ ok: false, message: "no element matched selector" }};
    el.click();
    return {{ ok: true, url: location.href }};
  }} catch (e) {{
    return {{ ok: false, message: String(e && e.message ? e.message : e) }};
  }}
}})()"#,
        sel = js_literal(selector)
    )
}

/// Type script: focus `selector` and set its value to `text`, dispatching
/// `input`/`change` so page frameworks observe the edit.
fn type_js(selector: &str, text: &str) -> String {
    format!(
        r#"(function () {{
  try {{
    var el = document.querySelector({sel});
    if (!el) return {{ ok: false, message: "no element matched selector" }};
    el.focus();
    if ("value" in el) {{
      el.value = {txt};
      el.dispatchEvent(new Event("input", {{ bubbles: true }}));
      el.dispatchEvent(new Event("change", {{ bubbles: true }}));
    }} else {{
      el.textContent = {txt};
    }}
    return {{ ok: true, url: location.href }};
  }} catch (e) {{
    return {{ ok: false, message: String(e && e.message ? e.message : e) }};
  }}
}})()"#,
        sel = js_literal(selector),
        txt = js_literal(text)
    )
}

/// Wrap a raw agent-supplied JS expression so a thrown exception becomes a
/// returned `{ __error }` instead of a swallowed callback (see module doc).
fn eval_js(js: &str) -> String {
    format!(
        r#"(function () {{
  try {{
    return (function () {{ return ({js}); }})();
  }} catch (e) {{
    return {{ __error: String(e && e.message ? e.message : e) }};
  }}
}})()"#
    )
}

// ── Tauri-backed page tools ─────────────────────────────────────────────────

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

/// Run one `eval_with_callback` round trip and parse its JSON result. Bridges
/// the `Fn(String)` callback to async via a oneshot; the `Mutex<Option<_>>`
/// lets the (multiply-callable, `'static`) callback consume the sender once.
async fn eval_value(view: &Webview, js: String) -> Result<serde_json::Value, BrowserError> {
    let (tx, rx) = oneshot::channel::<String>();
    let slot = Mutex::new(Some(tx));
    view.eval_with_callback(js, move |result: String| {
        if let Ok(mut guard) = slot.lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(result);
            }
        }
    })
    .map_err(|e| BrowserError::Webview(e.to_string()))?;

    let raw = tokio::time::timeout(EVAL_TIMEOUT, rx)
        .await
        .map_err(|_| BrowserError::Timeout)?
        .map_err(|_| BrowserError::Webview("eval callback was dropped".into()))?;

    serde_json::from_str(&raw)
        .map_err(|e| BrowserError::Webview(format!("eval result was not JSON ({e}): {raw}")))
}

/// Fail if the injected script reported an in-page `{ __error }`.
fn reject_page_error(v: &serde_json::Value) -> Result<(), BrowserError> {
    if let Some(msg) = v.get("__error").and_then(|e| e.as_str()) {
        return Err(BrowserError::Page(msg.to_owned()));
    }
    Ok(())
}

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
    // Visibility is owned by the frontend (set_visible on the Browser tab's
    // mount/unmount). Always create hidden so a background agent-initiated open
    // never paints over whatever tab the human is on; the mounted Browser tab
    // shows it via set_visible.
    let _ = view.hide();
    Ok(state_from(&view))
}

/// Navigate the current browser window. Errors with [`BrowserError::NotOpen`]
/// if no browser is open (use `open` to create one).
pub async fn goto(app: &AppHandle, url: &str) -> Result<BrowserState, BrowserError> {
    let target = normalize_url(url)?;
    let view = require_webview(app)?;
    view.navigate(target).map_err(|e| BrowserError::Webview(e.to_string()))?;
    Ok(state_from(&view))
}

/// Report the current URL/title, or a graceful `ok:false` when nothing is open.
pub async fn status(app: &AppHandle) -> Result<BrowserState, BrowserError> {
    match webview(app) {
        Some(view) => Ok(state_from(&view)),
        None => Ok(BrowserState {
            ok: false,
            url: None,
            title: None,
            message: Some("no browser is open".into()),
        }),
    }
}

/// DOM/text snapshot of the current page (capped body text).
pub async fn snapshot(app: &AppHandle, max_text: Option<i64>) -> Result<BrowserSnapshot, BrowserError> {
    let view = require_webview(app)?;
    let value = eval_value(&view, snapshot_js(clamp_max_text(max_text))).await?;
    reject_page_error(&value)?;
    serde_json::from_value(value)
        .map_err(|e| BrowserError::Webview(format!("snapshot shape mismatch: {e}")))
}

/// Click the element matching `selector` (selector as emitted by `snapshot`).
pub async fn click(app: &AppHandle, selector: &str) -> Result<BrowserActionResult, BrowserError> {
    let view = require_webview(app)?;
    let value = eval_value(&view, click_js(selector)).await?;
    reject_page_error(&value)?;
    serde_json::from_value(value)
        .map_err(|e| BrowserError::Webview(format!("click result shape mismatch: {e}")))
}

/// Focus and fill the input-like element matching `selector` with `text`.
pub async fn type_text(
    app: &AppHandle,
    selector: &str,
    text: &str,
) -> Result<BrowserActionResult, BrowserError> {
    let view = require_webview(app)?;
    let value = eval_value(&view, type_js(selector, text)).await?;
    reject_page_error(&value)?;
    serde_json::from_value(value)
        .map_err(|e| BrowserError::Webview(format!("type result shape mismatch: {e}")))
}

/// Escape hatch: evaluate `js` in the page and return its JSON result. Local
/// tool only — never exposed over any network or plugin passthrough.
pub async fn eval_json(app: &AppHandle, js: &str) -> Result<serde_json::Value, BrowserError> {
    let view = require_webview(app)?;
    let value = eval_value(&view, eval_js(js)).await?;
    reject_page_error(&value)?;
    Ok(value)
}

/// Close the browser window. Idempotent — closing when nothing is open is a
/// graceful `ok:true`.
pub async fn close(app: &AppHandle) -> Result<BrowserState, BrowserError> {
    if let Some(view) = webview(app) {
        view.close().map_err(|e| BrowserError::Webview(e.to_string()))?;
    }
    Ok(BrowserState { ok: true, url: None, title: None, message: None })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_url_fills_missing_scheme_with_https() {
        assert_eq!(normalize_url("example.com").unwrap().as_str(), "https://example.com/");
        assert_eq!(
            normalize_url("example.com/a/b?q=1").unwrap().as_str(),
            "https://example.com/a/b?q=1"
        );
    }

    #[test]
    fn normalize_url_preserves_explicit_and_special_schemes() {
        assert_eq!(normalize_url("http://x.test/").unwrap().as_str(), "http://x.test/");
        assert_eq!(normalize_url("https://x.test/").unwrap().as_str(), "https://x.test/");
        assert_eq!(normalize_url("about:blank").unwrap().as_str(), "about:blank");
        assert_eq!(
            normalize_url("file:///Users/x/page.html").unwrap().as_str(),
            "file:///Users/x/page.html"
        );
    }

    #[test]
    fn normalize_url_trims_and_rejects_empty() {
        assert_eq!(normalize_url("  example.com  ").unwrap().as_str(), "https://example.com/");
        assert!(matches!(normalize_url("   "), Err(BrowserError::InvalidUrl(_))));
        assert!(matches!(normalize_url(""), Err(BrowserError::InvalidUrl(_))));
    }

    #[test]
    fn normalize_url_rejects_unparseable() {
        // A space in the authority survives the https:// prefix and fails to
        // parse — must fail loudly, not load a bogus page.
        assert!(matches!(normalize_url("ht tp://%%%"), Err(BrowserError::InvalidUrl(_))));
    }

    #[test]
    fn clamp_max_text_defaults_and_caps() {
        assert_eq!(clamp_max_text(None), DEFAULT_MAX_TEXT);
        assert_eq!(clamp_max_text(Some(0)), DEFAULT_MAX_TEXT);
        assert_eq!(clamp_max_text(Some(-5)), DEFAULT_MAX_TEXT);
        assert_eq!(clamp_max_text(Some(500)), 500);
        assert_eq!(clamp_max_text(Some(999_999)), HARD_MAX_TEXT);
    }

    #[test]
    fn snapshot_js_embeds_the_cap_and_is_exception_safe() {
        let js = snapshot_js(4242);
        assert!(js.contains("var MAX = 4242;"), "resolved cap must be inlined");
        assert!(js.contains("__error"), "must return an error object, never throw");
        assert!(!js.contains("__MAX_TEXT__"), "placeholder must be fully replaced");
    }

    #[test]
    fn click_and_type_js_escape_the_selector_and_text() {
        // A selector containing a double-quote must be JSON-escaped so it can
        // never break out of the injected script.
        let js = click_js("a[href=\"x\"]");
        assert!(js.contains(r#"document.querySelector("a[href=\"x\"]")"#));

        let ty = type_js("#in", "he said \"hi\"\nline2");
        assert!(ty.contains(r##"querySelector("#in")"##));
        assert!(ty.contains(r#""he said \"hi\"\nline2""#), "text must be JSON-escaped");
        assert!(ty.contains("dispatchEvent"), "must notify page frameworks of the edit");
    }

    #[test]
    fn eval_js_wraps_expression_in_try_catch() {
        let js = eval_js("document.title");
        assert!(js.contains("document.title"));
        assert!(js.contains("__error"), "raw eval must not throw past the callback");
    }

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

    #[test]
    fn resolve_capture_size_defaults_and_clamps() {
        assert_eq!(resolve_capture_size(None, None), (1280.0, 800.0));
        assert_eq!(resolve_capture_size(Some(1440.0), Some(900.0)), (1440.0, 900.0));
        // below-min clamps up to 1, above-max clamps down to 10000
        assert_eq!(resolve_capture_size(Some(0.0), Some(-4.0)), (1.0, 1.0));
        assert_eq!(resolve_capture_size(Some(99999.0), None), (10000.0, 800.0));
    }
}

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

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewBuilder, WebviewUrl,
};
use tokio::sync::oneshot;

use super::browser_tabs::{BrowserOwner, BrowserState, TabId, TabRegistry};

/// Label PREFIX for the per-tab native webviews. Each tab's webview is labelled
/// `agent-browser:<tabId>` (see [`label_for`]); the old singleton used the bare
/// prefix. One browser overlay per Conclave app process, now multiplexed into N
/// concurrent tabs (only the active one visible; the rest hidden-but-alive).
const BROWSER_LABEL: &str = "agent-browser";

/// Round-trip budget for an `eval_with_callback` page tool. A remote page can
/// hang or block automation; we never wait forever — [`BrowserError::Timeout`]
/// surfaces a clear failure instead.
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `click`/`type` wait for their selector to appear before giving up,
/// and how often they re-check. An SPA that has not mounted the node yet used to
/// answer "no element matched selector" instantly, which reads to an agent as
/// "click is broken" — waiting is what a human does without thinking.
const DEFAULT_SELECTOR_TIMEOUT_MS: u64 = 5_000;
const SELECTOR_POLL: Duration = Duration::from_millis(100);

/// Snapshot body-text cap (plan §Runtime Notes): default and hard maximum. The
/// cap keeps a large page from flooding an agent's context window.
const DEFAULT_MAX_TEXT: usize = 12_000;
const HARD_MAX_TEXT: usize = 50_000;

/// Offscreen anchor for a browser opened before React has reported its region
/// rectangle — keeps the native overlay from flashing over the app chrome until
/// the first `set_bounds` positions it.
const OFFSCREEN: f64 = -10_000.0;

/// Where a concealed tab is parked. Far enough outside the window that it can
/// never be seen, while still being a SHOWN webview — see [`conceal`].
const PARK: f64 = -20_000.0;
/// Size a parked tab is given when no region rect has ever been reported. A
/// degenerate 1x1 would give a page nothing to lay out; this is an ordinary
/// viewport so background work behaves the way it will once revealed.
const PARK_FALLBACK: LogicalSize<f64> = LogicalSize::new(DEFAULT_CAPTURE_W, DEFAULT_CAPTURE_H);

/// Round-trip budget for a `takeSnapshot` capture (renders + PNG encode).
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(15);

/// Default capture viewport when the caller gives no size (logical px).
const DEFAULT_CAPTURE_W: f64 = 1280.0;
const DEFAULT_CAPTURE_H: f64 = 800.0;
/// Hard bounds so a bad `--width/--height` can't ask for a 0-px or absurd canvas.
const MIN_CAPTURE_PX: f64 = 1.0;
const MAX_CAPTURE_PX: f64 = 10_000.0;

// ── Result types (mirrored 1:1 by src/ipc/types.ts, camelCase) ──────────────
//
// The tab-list reply shape (`BrowserState`/`BrowserTab`/`BrowserOwner`) lives in
// `super::browser_tabs` — the pure, unit-tested registry owns the wire contract.
// The action/snapshot/shot result types below stay here (they belong to the
// page tools, not the tab registry).

/// The viewport rectangle the Browser tab reserves for the native overlay. It
/// is DEFINED in `browser_tabs` (the registry stores the last reported rect and
/// decides where a created-or-revealed webview lands) and re-exported here so
/// `commands::browser` keeps importing it from `runtime::browser` unchanged.
pub use super::browser_tabs::Bounds;

/// Result of an in-page action (`click`/`type`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Tag name of the element that was actually hit, for the agent's log — a
    /// selector matching the wrong node is the other half of "click does not
    /// work". Absent on a miss, and omitted from JSON when absent, so the
    /// frontend's `BrowserActionResult` mirror keeps deserializing unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// First 80 characters of the hit element's text, same purpose as `tag`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Agent-facing DOM/text snapshot (NOT pixels — see plan non-goals).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSnapshot {
    pub url: String,
    pub title: String,
    /// `document.readyState`. An agent blaming `click` is often looking at a
    /// page that never finished loading — say so instead of making them guess.
    #[serde(default)]
    pub ready_state: String,
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

/// Result of `ping` — a cheap "is this page alive" probe an agent can run BEFORE
/// blaming `click`. `raf_ticks_per_200ms` is 0 in a webview that is not painting,
/// which is the honest answer to "why did nothing happen": the page's own
/// rAF-driven work (React concurrent rendering, base-ui transitions) is frozen,
/// not the click.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPing {
    pub ready_state: String,
    pub raf_ticks_per_200ms: u64,
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
            BrowserError::NotOpen => {
                write!(f, "no browser is open — run `browser open <url>` first")
            }
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

/// Resolve the caller's selector timeout: absent → [`DEFAULT_SELECTOR_TIMEOUT_MS`],
/// `Some(0)` → a single attempt with no wait, otherwise the requested budget.
fn resolve_selector_timeout(requested: Option<u64>) -> u64 {
    requested.unwrap_or(DEFAULT_SELECTOR_TIMEOUT_MS)
}

/// Whether an action result is the retryable "selector matched nothing" miss, as
/// opposed to a real in-page failure (which must surface immediately).
fn is_selector_miss(v: &serde_json::Value) -> bool {
    v.get("ok").and_then(|o| o.as_bool()) == Some(false)
        && v.get("message").and_then(|m| m.as_str()) == Some(SELECTOR_MISS)
}

/// The result returned once the wait budget is spent — names the budget and the
/// selector, so an agent can tell "wrong selector" from "page was too slow".
fn selector_timeout_result(timeout_ms: u64, selector: &str) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "message": format!("selector not found within {timeout_ms}ms: {selector}"),
    })
}

/// Run `attempt` until it stops reporting a selector miss, or the budget runs
/// out. The retry lives in RUST, not in the injected script, because
/// `eval_with_callback` hands back the completion value of a synchronous
/// expression — it cannot await a promise, so the JS can never poll for itself.
///
/// `timeout_ms == 0` means a single attempt. Any non-miss result (a hit, or a
/// real in-page error) returns straight away.
async fn wait_for_selector<F, Fut>(
    timeout_ms: u64,
    selector: &str,
    mut attempt: F,
) -> Result<serde_json::Value, BrowserError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, BrowserError>>,
{
    let first = attempt().await?;
    if timeout_ms == 0 || !is_selector_miss(&first) {
        return Ok(first);
    }
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if std::time::Instant::now() >= deadline {
            return Ok(selector_timeout_result(timeout_ms, selector));
        }
        tokio::time::sleep(SELECTOR_POLL).await;
        let next = attempt().await?;
        if !is_selector_miss(&next) {
            return Ok(next);
        }
    }
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
      readyState: document.readyState,
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

/// The message the injected `click`/`type` scripts return when `querySelector`
/// matched nothing. It is the ONE `ok:false` the wait loop retries on — every
/// other failure is real and returns at once — so the JS text and this constant
/// must stay in lockstep.
const SELECTOR_MISS: &str = "no element matched selector";

/// Click script for `selector`: a REAL pointer sequence at the element's centre,
/// not `el.click()`.
///
/// `el.click()` fires a lone synthetic `click`, which React/base-ui/Radix
/// widgets ignore — they listen on `pointerdown`/`mousedown`, which is why
/// menus, popovers and toggles could not be driven from the CLI at all and
/// agents escaped to Chrome instead. So: scroll the element to the middle of the
/// viewport, take its centre, and dispatch the full sequence a real mouse
/// produces. `el.click()` remains only as a fallback if the dispatch throws (an
/// exotic element, or a browser without `PointerEvent`).
///
/// `isTrusted` is still false on these events — a site that gates on it needs
/// CDP-level input, which is out of scope (plan §Risks).
///
/// Returns `{ok:true, url, tag, text}` on a hit, `{ok:false, message}` when the
/// selector matches nothing.
fn click_js(selector: &str) -> String {
    format!(
        r#"(function () {{
  try {{
    var el = document.querySelector({sel});
    if (!el) return {{ ok: false, message: {miss} }};
    try {{ el.scrollIntoView({{ block: "center", inline: "center" }}); }} catch (e) {{}}
    var r = el.getBoundingClientRect();
    var cx = r.left + r.width / 2;
    var cy = r.top + r.height / 2;
    function init(buttons) {{
      return {{
        bubbles: true, cancelable: true, composed: true, view: window, detail: 1,
        clientX: cx, clientY: cy, screenX: cx, screenY: cy,
        button: 0, buttons: buttons
      }};
    }}
    function pointer(type, buttons) {{
      var o = init(buttons);
      o.pointerId = 1; o.pointerType = "mouse"; o.isPrimary = true;
      o.width = 1; o.height = 1; o.pressure = buttons ? 0.5 : 0;
      return new PointerEvent(type, o);
    }}
    function mouse(type, buttons) {{ return new MouseEvent(type, init(buttons)); }}
    var real = false;
    try {{
      el.dispatchEvent(pointer("pointerover", 0));
      el.dispatchEvent(pointer("pointerenter", 0));
      el.dispatchEvent(mouse("mouseover", 0));
      el.dispatchEvent(pointer("pointermove", 0));
      el.dispatchEvent(mouse("mousemove", 0));
      el.dispatchEvent(pointer("pointerdown", 1));
      el.dispatchEvent(mouse("mousedown", 1));
      if (typeof el.focus === "function") {{
        try {{ el.focus({{ preventScroll: true }}); }} catch (e) {{ try {{ el.focus(); }} catch (e2) {{}} }}
      }}
      el.dispatchEvent(pointer("pointerup", 0));
      el.dispatchEvent(mouse("mouseup", 0));
      el.dispatchEvent(mouse("click", 0));
      real = true;
    }} catch (e) {{}}
    if (!real) el.click();
    var text = "";
    try {{
      text = (el.innerText || el.textContent || "").replace(/\s+/g, " ").trim().slice(0, 80);
    }} catch (e) {{}}
    return {{ ok: true, url: location.href, tag: el.tagName.toLowerCase(), text: text }};
  }} catch (e) {{
    return {{ ok: false, message: String(e && e.message ? e.message : e) }};
  }}
}})()"#,
        sel = js_literal(selector),
        miss = js_literal(SELECTOR_MISS)
    )
}

/// Type script: focus `selector` and set its value to `text`, dispatching
/// `input`/`change` so page frameworks observe the edit.
fn type_js(selector: &str, text: &str) -> String {
    format!(
        r#"(function () {{
  try {{
    var el = document.querySelector({sel});
    if (!el) return {{ ok: false, message: {miss} }};
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
        txt = js_literal(text),
        miss = js_literal(SELECTOR_MISS)
    )
}

/// Wrap raw agent-supplied JS so a thrown exception becomes a returned
/// `{ __error }` instead of a swallowed callback (see module doc). The source
/// is embedded as a string literal and compiled INSIDE the page — so
/// multi-statement input can never turn the whole wrapper into a parse-time
/// SyntaxError (which would bypass the try/catch entirely). Expression-first
/// via `new Function("return (…)")` (so `{a:1}` stays an object literal, not a
/// block); multi-statement input falls back to indirect `eval`, the only form
/// with completion-value semantics — a plain `Function` body without `return`
/// would run the statements but answer `undefined` (challenge 92aa7d2b).
/// Construction and execution are separated so a RUNTIME throw from the
/// expression path propagates to the outer catch instead of re-running the
/// source's side effects through the fallback.
///
/// The completion value is forced JSON-safe via a `JSON.stringify` round trip
/// BEFORE it reaches the native bridge: WebKit marshals a `Date` result to
/// `NSDate` and non-finite numbers to non-finite `NSNumber`s, and wry's
/// completion handler feeds the marshalled value straight into
/// `NSJSONSerialization`, which throws `NSInvalidArgumentException` for both
/// ("Invalid type in JSON write (__NSTaggedDate)") — an uncaught ObjC
/// exception that kills the whole app and never reaches the Rust panic hook
/// (task browser-eval-json-safety, reproduced 2026-07-11 with `new Date()`
/// and `NaN`). After the round trip a Date answers as its ISO string,
/// NaN/Infinity as null, unstringifiable values (BigInt, cycles) as
/// `{ __error }` via the outer catch, and a value stringify erases entirely
/// (function, symbol) as null.
fn eval_js(js: &str) -> String {
    format!(
        r#"(function () {{
  try {{
    var src = {src};
    var fn = null;
    try {{
      fn = new Function("return (" + src + "\n)");
    }} catch (e) {{}}
    var r = fn ? fn() : (0, eval)(src);
    if (r === undefined) return null;
    var s = JSON.stringify(r);
    return s === undefined ? null : JSON.parse(s);
  }} catch (e) {{
    return {{ __error: String(e && e.message ? e.message : e) }};
  }}
}})()"#,
        src = js_literal(js)
    )
}

// ── Tab registry + native webview pool ───────────────────────────────────────

/// The process-global tab registry: all per-tab metadata (owner, last-navigated
/// url, title, loading, ended), the active-tab pointer, and the human-tab
/// sequence. The native webviews themselves live in Tauri's own manager, keyed
/// by [`label_for`]; this registry holds everything Tauri does not. One browser
/// overlay per app process (mirrors the old singleton), now multiplexed.
fn registry() -> &'static Mutex<TabRegistry> {
    static REG: OnceLock<Mutex<TabRegistry>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(TabRegistry::new()))
}

/// Lock the registry, recovering from a poisoned mutex rather than cascading a
/// panic (matches the runtime module's lock convention). The closure runs while
/// the lock is held — callers MUST NOT `.await` inside it (the guard is a
/// `std::sync::Mutex`, not held across suspension points anywhere below).
fn with_registry<R>(f: impl FnOnce(&mut TabRegistry) -> R) -> R {
    let mut guard = registry().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// Native webview label for a tab: `agent-browser:<tabId>`. Tauri v2 permits
/// `-`, `:` and `_` in labels; every `tabId` is either an agent UUID or
/// `human-<seq>`, both already label-safe, so no sanitization is needed.
fn label_for(tab_id: &str) -> String {
    format!("{BROWSER_LABEL}:{tab_id}")
}

/// The native webview for `tab_id`, if one has been created (a human tab exists
/// in the registry before its first navigation and so has none yet).
fn webview_for(app: &AppHandle, tab_id: &str) -> Option<Webview> {
    app.get_webview(&label_for(tab_id))
}

fn require_webview_for(app: &AppHandle, tab_id: &str) -> Result<Webview, BrowserError> {
    webview_for(app, tab_id).ok_or(BrowserError::NotOpen)
}

// ── Registry-only operations (no native webview) ─────────────────────────────

/// The full tab list + active pointer — the `browser.status` reply. A pure read
/// of the registry: it NEVER probes a native URL (`WKWebView.URL` on a fresh
/// child webview panics on the main thread and kills the whole app — crash
/// history `de0a632f`); every tab's `url` is the caller's navigation target,
/// stored on navigate.
pub fn state() -> BrowserState {
    with_registry(|r| r.state())
}

/// Open a fresh HUMAN tab (registry-only — its native webview is created lazily
/// on the first navigation). Returns the new tabId (`human-<seq>`).
pub fn new_human_tab() -> TabId {
    with_registry(|r| r.new_human_tab())
}

/// Mark an agent's tab `ended` (read-only until the human closes it, D4b). The
/// webview stays ALIVE — the human can still view the final page after the agent
/// is gone; only the human's `close` tears it down.
///
/// Registry-only and INTENTIONALLY app-less: the frontend `InAppBrowserView`
/// polls `browser.status` every 2s, so flipping the registry flag is enough to
/// repaint the ended badge — there is NO browser-state event to emit (every
/// change, incl. `navigate`/`set_active`, propagates via that poll). Wired into
/// the terminal agent-exit paths in `instance.rs` (crash-EOF / `stop` /
/// `remove`); `restart` deliberately does NOT call it (D-1: the tab is reused by
/// the respawned generation). A no-op for a human/unknown tab, and idempotent.
pub fn mark_ended(agent_id: &str) {
    with_registry(|r| r.mark_ended(agent_id));
}

/// Clear an agent tab's ended marker after a fresh runtime generation has
/// registered successfully. Registry-only; the frontend's status poll observes
/// the update, matching [`mark_ended`].
pub fn mark_resumed(agent_id: &str) {
    with_registry(|r| r.mark_resumed(agent_id));
}

/// Test-only: seed an AGENT tab straight into the process-global registry
/// (app-less, no native webview) so lifecycle tests in `instance.rs` can assert
/// `mark_ended` flips it. Callers MUST use a process-unique `agent_id` (the
/// registry is a shared `static`) — the fixture UUIDs already satisfy this.
#[cfg(test)]
pub(crate) fn test_seed_agent_tab(agent_id: &str) {
    use super::browser_tabs::OwnerKind;
    with_registry(|r| {
        r.upsert(
            agent_id.to_string(),
            BrowserOwner {
                kind: OwnerKind::Agent,
                id: agent_id.to_string(),
                label: agent_id.to_string(),
            },
            Some("https://example.test/".to_string()),
        );
    });
}

// ── Native webview pool (the tab-keyed multiplex) ────────────────────────────

/// Create-or-reuse the tab keyed by `tab_id` and navigate it to `url`.
///
/// On CREATE: records `owner`, then `add_child`s the native webview at the rect
/// the registry resolves — `bounds` when the caller has one, else the last rect
/// the frontend reported, else offscreen. It is SHOWN only when it is the active
/// tab and the Browser view has the overlay on screen; otherwise it stays hidden
/// until [`set_active`]/[`set_visible`] reveals it, so a background
/// agent-initiated open never paints over whatever tab the human is on. On
/// REUSE: navigates the existing webview, owner untouched. Loading is cleared
/// once the navigation is dispatched.
///
/// Showing here is what makes the FIRST page paint: before, every fresh webview
/// was created hidden and offscreen, so the human had to switch tabs and back
/// for the overlay to appear (task `browser-first-paint`).
///
/// NEVER reads the native URL — `url` (the caller's target) is what the registry
/// stores and what `status` returns (crash history `de0a632f`).
pub fn navigate(
    app: &AppHandle,
    tab_id: &str,
    owner: BrowserOwner,
    url: &str,
    bounds: Option<Bounds>,
) -> Result<BrowserState, BrowserError> {
    let target = normalize_url(url)?;
    with_registry(|r| {
        r.upsert(tab_id.to_string(), owner, Some(target.to_string()));
    });
    match webview_for(app, tab_id) {
        Some(view) => {
            view.navigate(target)
                .map_err(|e| BrowserError::Webview(e.to_string()))?;
        }
        None => {
            let window = app
                .get_window("main")
                .ok_or_else(|| BrowserError::Webview("main window not found".into()))?;
            // The registry decides both halves: WHERE (the caller's rect, else
            // the last one the frontend reported) and WHETHER it may paint (only
            // the active tab, only while the overlay is on screen).
            let placement = with_registry(|r| r.placement_for_create(tab_id, bounds));
            let (position, size) = resolve_bounds(placement.bounds);
            let view = window
                .add_child(
                    WebviewBuilder::new(label_for(tab_id), WebviewUrl::External(target)),
                    position,
                    size,
                )
                .map_err(|e| BrowserError::Webview(e.to_string()))?;
            // `add_child` already placed it, so showing here cannot flash the
            // page at the offscreen default. Anything not showable is CONCEALED
            // (parked offscreen, still ticking) until `set_active`/`set_visible`
            // reveals it.
            let _ = if placement.show {
                view.show()
                    .map_err(|e| BrowserError::Webview(e.to_string()))
            } else {
                conceal(&view, placement.bounds)
            };
        }
    }
    // v1 has no native load-complete signal, so clear `loading` once the
    // navigation is dispatched rather than leave a perpetual spinner in the
    // rail. The `loading` field stays in the contract for a future load hook.
    with_registry(|r| r.set_loaded(tab_id, None));
    Ok(state())
}

/// Position and size `view` over `bounds`. Every reveal path calls this first: a
/// hidden webview keeps the frame it was last given, so a window resize that
/// happened while another tab was active would otherwise leave the incoming page
/// misplaced.
fn apply_bounds(view: &Webview, bounds: Bounds) -> Result<(), BrowserError> {
    let (position, size) = resolve_bounds(Some(bounds));
    view.set_position(position)
        .map_err(|e| BrowserError::Webview(e.to_string()))?;
    view.set_size(size)
        .map_err(|e| BrowserError::Webview(e.to_string()))?;
    Ok(())
}

/// Conceal `view` — the counterpart to [`reveal_at`], and the ONE place that
/// decides what "hidden" means natively.
///
/// EXPERIMENT (plan Decision 4): instead of `view.hide()`, park the webview
/// offscreen but SHOWN. A hidden WebKit webview stops ticking —
/// `requestAnimationFrame` never fires (Dew measured `rafTicks=0` after 1.5 s on
/// task roster-row-supervisor-trash) — so a background agent's page freezes:
/// React concurrent rendering, base-ui transitions and any rAF-driven work stop
/// dead, and every one of those reads to an agent as "click did nothing". An
/// agent's tab is almost never the active one, so this is the common case, not
/// the edge case. A parked webview stays in-window, so WebKit keeps scheduling
/// frames for it.
///
/// It cannot paint over the app chrome at `PARK`: Decision 7's guarantee (no
/// webview visible while the Browser view is unmounted) is preserved by
/// position, not by the hidden flag.
///
/// If the measured gate (rAF ticks from a background tab) shows parking does NOT
/// keep frames alive on this macOS/WebKit, the documented fallback is to make
/// this function `view.hide()` again — nothing else changes.
fn conceal(view: &Webview, bounds: Option<Bounds>) -> Result<(), BrowserError> {
    let size = bounds
        .map(|b| LogicalSize::new(b.width.max(0.0), b.height.max(0.0)))
        .filter(|s: &LogicalSize<f64>| s.width > 0.0 && s.height > 0.0)
        .unwrap_or(PARK_FALLBACK);
    view.set_position(LogicalPosition::new(PARK, PARK))
        .map_err(|e| BrowserError::Webview(e.to_string()))?;
    view.set_size(size)
        .map_err(|e| BrowserError::Webview(e.to_string()))?;
    view.show()
        .map_err(|e| BrowserError::Webview(e.to_string()))
}

/// Reveal `view` at `bounds`: position FIRST, then show — and never show when
/// the positioning failed. A webview shown after a failed `set_position`/
/// `set_size` paints at whatever frame it happened to be holding: the offscreen
/// default for a fresh one, a pre-resize rect for a hidden one. Every reveal
/// path funnels through here and propagates the failure, so a caller can never
/// mistake a half-applied reveal for a good one (Armin, review of a679e3f).
fn reveal_at(view: &Webview, bounds: Option<Bounds>) -> Result<(), BrowserError> {
    if let Some(bounds) = bounds {
        apply_bounds(view, bounds)?;
    }
    view.show()
        .map_err(|e| BrowserError::Webview(e.to_string()))
}

/// Make `tab_id` the visible tab. Hides every OTHER live webview FIRST, then
/// reveals this one — the hide-before-show order is load-bearing (invariant #4:
/// otherwise the outgoing page flashes over the incoming one during a switch).
/// The reveal repositions to the last reported rect first, so the invariant holds
/// no matter who called (`browser.setActive` over the engine socket reports no
/// rect of its own).
///
/// While the Browser view is UNMOUNTED this is registry-only: the active pointer
/// moves and no webview is touched, because showing one would paint it over the
/// app chrome (Decision 7). A no-op-safe switch to a tab whose webview does not
/// exist yet (a new human tab before its first navigation). Returns the updated
/// tab list.
pub fn set_active(app: &AppHandle, tab_id: &str) -> Result<BrowserState, BrowserError> {
    let activation = with_registry(|r| r.activate(tab_id));
    if !activation.known {
        return Ok(state());
    }
    for id in &activation.hide {
        if let Some(view) = webview_for(app, id) {
            let _ = conceal(&view, activation.incoming.bounds);
        }
    }
    if activation.incoming.show {
        if let Some(view) = webview_for(app, tab_id) {
            reveal_at(&view, activation.incoming.bounds)?;
        }
    }
    Ok(state())
}

/// Position/resize the ACTIVE tab's webview over the Browser tab's reserved
/// region, and REMEMBER the rect: it is the one every later create or reveal
/// falls back to, which is why a rect reported before any webview exists is no
/// longer lost. A graceful no-op when there is no active tab or it has no
/// webview yet. Inactive webviews are repositioned when they are revealed, not
/// here (they are hidden anyway).
pub fn set_bounds(app: &AppHandle, bounds: Bounds) -> Result<(), BrowserError> {
    let active = with_registry(|r| {
        r.set_last_bounds(bounds);
        r.state().active_tab_id
    });
    if let Some(id) = active {
        if let Some(view) = webview_for(app, &id) {
            apply_bounds(&view, bounds)?;
        }
    }
    Ok(())
}

/// Show/hide the whole browser overlay (the ACTIVE tab's webview) on the Browser
/// tab's mount/unmount, WITHOUT closing it — background agents keep driving
/// hidden tabs. The flag is RECORDED on the registry: a webview created later
/// while the overlay is off screen must not paint, and one created while it is on
/// screen must. Showing repositions first (the webview may have been hidden
/// across a resize). A graceful no-op when nothing is active.
pub fn set_visible(app: &AppHandle, visible: bool) -> Result<(), BrowserError> {
    let target = with_registry(|r| {
        r.set_overlay_visible(visible);
        r.active_placement()
    });
    // `placement.show` mirrors the flag just recorded, so this IS the caller's
    // `visible` — reading it from the registry keeps one source of truth.
    if let Some((id, placement)) = target {
        if let Some(view) = webview_for(app, &id) {
            if placement.show {
                reveal_at(&view, placement.bounds)?;
            } else {
                conceal(&view, placement.bounds)?;
            }
        }
    }
    Ok(())
}

/// Close ONE tab: destroy its native webview (if any) and drop it from the
/// registry, which reselects the first remaining tab as active. Reveals that new
/// active tab — while the Browser view is on screen — so the overlay never goes
/// blank while a tab remains. That reveal is BEST-EFFORT (Decision 6): this call
/// reports whether the CLOSE succeeded, and a tab that closed cleanly must not
/// report failure because the tab behind it could not be shown. Returns the
/// updated tab list.
pub fn close_tab(app: &AppHandle, tab_id: &str) -> Result<BrowserState, BrowserError> {
    if let Some(view) = webview_for(app, tab_id) {
        view.close()
            .map_err(|e| BrowserError::Webview(e.to_string()))?;
    }
    let promoted = with_registry(|r| {
        r.close(tab_id);
        r.active_placement()
    });
    if let Some((id, placement)) = promoted {
        // Same reveal rule as `set_active`: reposition first (the promoted tab
        // has been hidden, so its frame may predate the current window size),
        // and reveal only while the Browser view is on screen.
        //
        // INVARIANT (Decision 6): this call REPORTS THE CLOSE, not the
        // promotion. The tab is already destroyed and dropped from the registry
        // by the time we get here, so a failed reveal must NOT become an `Err` —
        // the frontend would show "Couldn't close the tab" (false) and skip
        // `applyState`, leaving the closed tab in the rail until the 2 s poll.
        // Best-effort plus a warn; only the target's own `view.close()` above
        // keeps its `?`. `reveal_at` still never shows after a failed position.
        //
        // Not unit-testable: `reveal_at` is native (`set_position`/`show`), so
        // this invariant lives in the comment, per Decision 6's own fallback.
        if placement.show {
            if let Some(view) = webview_for(app, &id) {
                if let Err(e) = reveal_at(&view, placement.bounds) {
                    eprintln!("[browser] tab {id} was promoted but could not be revealed: {e}");
                }
            }
        }
    }
    Ok(state())
}

// ── Tauri-backed page tools (per-tab) ────────────────────────────────────────

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

    // Defense in depth: an empty callback payload means the injected script
    // itself failed to run (e.g. a wrapper-level parse error) — say so instead
    // of surfacing serde's bare EOF text.
    if raw.trim().is_empty() {
        return Err(BrowserError::Webview(
            "eval produced no result (script failed to parse?)".into(),
        ));
    }
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

/// Window over which [`ping`] counts animation frames. Short enough to stay a
/// cheap probe, long enough that a painting webview (~60 fps) clears 10 ticks by
/// a wide margin while a frozen one stays at exactly 0.
const PING_WINDOW: Duration = Duration::from_millis(200);

/// Arm a frame counter on `window`, replacing any previous one. Split from
/// [`PING_READ_JS`] because `eval_with_callback` returns the completion value of
/// a SYNCHRONOUS expression: counting frames needs two round trips with real
/// time between them, which only Rust can arrange.
const PING_ARM_JS: &str = r#"(function () {
  try {
    window.__conclavePingTicks = 0;
    var step = function () {
      window.__conclavePingTicks++;
      window.__conclavePingRaf = requestAnimationFrame(step);
    };
    if (window.__conclavePingRaf) cancelAnimationFrame(window.__conclavePingRaf);
    window.__conclavePingRaf = requestAnimationFrame(step);
    return { ok: true };
  } catch (e) {
    return { __error: String(e && e.message ? e.message : e) };
  }
})()"#;

/// Read the frame count back, stop the counter, and report `readyState`.
const PING_READ_JS: &str = r#"(function () {
  try {
    if (window.__conclavePingRaf) cancelAnimationFrame(window.__conclavePingRaf);
    window.__conclavePingRaf = null;
    var n = window.__conclavePingTicks || 0;
    window.__conclavePingTicks = 0;
    return { readyState: document.readyState, rafTicksPer200ms: n };
  } catch (e) {
    return { __error: String(e && e.message ? e.message : e) };
  }
})()"#;

/// Probe tab `tab_id`: is the document loaded, and are animation frames actually
/// running? Two evals [`PING_WINDOW`] apart — arm a counter, wait, read it back.
pub async fn ping(app: &AppHandle, tab_id: &str) -> Result<BrowserPing, BrowserError> {
    let view = require_webview_for(app, tab_id)?;
    let armed = eval_value(&view, PING_ARM_JS.to_string()).await?;
    reject_page_error(&armed)?;
    tokio::time::sleep(PING_WINDOW).await;
    let value = eval_value(&view, PING_READ_JS.to_string()).await?;
    reject_page_error(&value)?;
    serde_json::from_value(value)
        .map_err(|e| BrowserError::Webview(format!("ping result shape mismatch: {e}")))
}

/// DOM/text snapshot of tab `tab_id`'s current page (capped body text).
pub async fn snapshot(
    app: &AppHandle,
    tab_id: &str,
    max_text: Option<i64>,
) -> Result<BrowserSnapshot, BrowserError> {
    let view = require_webview_for(app, tab_id)?;
    let value = eval_value(&view, snapshot_js(clamp_max_text(max_text))).await?;
    reject_page_error(&value)?;
    serde_json::from_value(value)
        .map_err(|e| BrowserError::Webview(format!("snapshot shape mismatch: {e}")))
}

/// Click the element matching `selector` in tab `tab_id` (selector as emitted
/// by `snapshot`), waiting up to `timeout_ms` for it to appear.
pub async fn click(
    app: &AppHandle,
    tab_id: &str,
    selector: &str,
    timeout_ms: Option<u64>,
) -> Result<BrowserActionResult, BrowserError> {
    let view = require_webview_for(app, tab_id)?;
    let js = click_js(selector);
    let value = wait_for_selector(resolve_selector_timeout(timeout_ms), selector, || {
        eval_value(&view, js.clone())
    })
    .await?;
    reject_page_error(&value)?;
    serde_json::from_value(value)
        .map_err(|e| BrowserError::Webview(format!("click result shape mismatch: {e}")))
}

/// Focus and fill the input-like element matching `selector` in tab `tab_id`
/// with `text`, waiting up to `timeout_ms` for the element to appear.
pub async fn type_text(
    app: &AppHandle,
    tab_id: &str,
    selector: &str,
    text: &str,
    timeout_ms: Option<u64>,
) -> Result<BrowserActionResult, BrowserError> {
    let view = require_webview_for(app, tab_id)?;
    let js = type_js(selector, text);
    let value = wait_for_selector(resolve_selector_timeout(timeout_ms), selector, || {
        eval_value(&view, js.clone())
    })
    .await?;
    reject_page_error(&value)?;
    serde_json::from_value(value)
        .map_err(|e| BrowserError::Webview(format!("type result shape mismatch: {e}")))
}

/// Escape hatch: evaluate `js` in tab `tab_id`'s page and return its JSON
/// result. Local tool only — never exposed over any network or plugin
/// passthrough.
pub async fn eval_json(
    app: &AppHandle,
    tab_id: &str,
    js: &str,
) -> Result<serde_json::Value, BrowserError> {
    let view = require_webview_for(app, tab_id)?;
    let value = eval_value(&view, eval_js(js)).await?;
    reject_page_error(&value)?;
    Ok(value)
}

/// One-shot slot carrying the encoded PNG (or an error string) from the
/// main-thread `takeSnapshot` completion block back to the async caller.
#[cfg(target_os = "macos")]
type ShotSlot = Mutex<Option<oneshot::Sender<Result<Vec<u8>, String>>>>;

/// Capture the embedded page to a PNG at `path` (absolute; the CLI resolves it
/// in the agent's cwd). Sizes the webview to the capture viewport, snapshots via
/// WKWebView.takeSnapshot, restores the prior size, and writes the PNG. Works
/// headless (the webview may be hidden/offscreen). macOS only.
#[cfg(target_os = "macos")]
pub async fn screenshot(
    app: &AppHandle,
    tab_id: &str,
    path: &str,
    width: Option<f64>,
    height: Option<f64>,
) -> Result<BrowserShot, BrowserError> {
    let view = require_webview_for(app, tab_id)?;
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
        use objc2::{AnyThread, MainThreadMarker};
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
        use objc2_foundation::{NSDictionary, NSNumber};
        use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

        // with_webview runs on the main thread; the WKWebView pointer comes from
        // the platform handle's inner().
        let send = |slot: &ShotSlot, v: Result<Vec<u8>, String>| {
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
                    let Some(rep) =
                        NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)
                    else {
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
                },
            );
            wk.takeSnapshotWithConfiguration_completionHandler(Some(&cfg), &handler);
        }
    });

    // Restore size regardless of how the snapshot went.
    let restore = |view: &Webview, prev: &Option<tauri::Rect>| {
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

    Ok(BrowserShot {
        path: path.to_owned(),
        width: w,
        height: h,
    })
}

/// Non-macOS: capture uses a macOS-only native API.
#[cfg(not(target_os = "macos"))]
pub async fn screenshot(
    _app: &AppHandle,
    _tab_id: &str,
    _path: &str,
    _width: Option<f64>,
    _height: Option<f64>,
) -> Result<BrowserShot, BrowserError> {
    Err(BrowserError::Webview(
        "screenshot is only supported on macOS".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T1: `mark_ended` is idempotent (two marks = one ended tab, no panic) and
    /// a silent no-op for an unknown id (creates nothing). Ids are process-unique
    /// because the registry is a shared `static` seen by every test in this bin.
    #[test]
    fn mark_ended_is_idempotent_and_noops_unknown_id() {
        let seeded = "t1-ended-idem-seeded";
        let unknown = "t1-ended-idem-unknown";

        test_seed_agent_tab(seeded);
        mark_ended(seeded);
        mark_ended(seeded); // second mark must be harmless.

        mark_ended(unknown); // no seeded tab → silent no-op, no panic.

        let tabs = state().tabs;
        let seeded_tab = tabs
            .iter()
            .find(|t| t.tab_id == seeded)
            .expect("seeded tab present");
        assert!(seeded_tab.ended, "seeded agent tab must be ended");
        assert!(
            !tabs.iter().any(|t| t.tab_id == unknown),
            "mark_ended must not create a tab for an unknown id"
        );
    }

    #[test]
    fn mark_resumed_clears_ended_and_noops_unknown_id() {
        let seeded = "t1-resumed-seeded";
        let unknown = "t1-resumed-unknown";

        test_seed_agent_tab(seeded);
        mark_ended(seeded);
        mark_resumed(seeded);
        mark_resumed(unknown);

        let tabs = state().tabs;
        let seeded_tab = tabs
            .iter()
            .find(|t| t.tab_id == seeded)
            .expect("seeded tab present");
        assert!(!seeded_tab.ended, "resumed agent tab must be live again");
        assert!(
            !tabs.iter().any(|t| t.tab_id == unknown),
            "mark_resumed must not create an unknown tab"
        );
    }

    #[test]
    fn normalize_url_fills_missing_scheme_with_https() {
        assert_eq!(
            normalize_url("example.com").unwrap().as_str(),
            "https://example.com/"
        );
        assert_eq!(
            normalize_url("example.com/a/b?q=1").unwrap().as_str(),
            "https://example.com/a/b?q=1"
        );
    }

    #[test]
    fn normalize_url_preserves_explicit_and_special_schemes() {
        assert_eq!(
            normalize_url("http://x.test/").unwrap().as_str(),
            "http://x.test/"
        );
        assert_eq!(
            normalize_url("https://x.test/").unwrap().as_str(),
            "https://x.test/"
        );
        assert_eq!(
            normalize_url("about:blank").unwrap().as_str(),
            "about:blank"
        );
        assert_eq!(
            normalize_url("file:///Users/x/page.html").unwrap().as_str(),
            "file:///Users/x/page.html"
        );
    }

    #[test]
    fn normalize_url_trims_and_rejects_empty() {
        assert_eq!(
            normalize_url("  example.com  ").unwrap().as_str(),
            "https://example.com/"
        );
        assert!(matches!(
            normalize_url("   "),
            Err(BrowserError::InvalidUrl(_))
        ));
        assert!(matches!(
            normalize_url(""),
            Err(BrowserError::InvalidUrl(_))
        ));
    }

    #[test]
    fn normalize_url_rejects_unparseable() {
        // A space in the authority survives the https:// prefix and fails to
        // parse — must fail loudly, not load a bogus page.
        assert!(matches!(
            normalize_url("ht tp://%%%"),
            Err(BrowserError::InvalidUrl(_))
        ));
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
        assert!(
            js.contains("var MAX = 4242;"),
            "resolved cap must be inlined"
        );
        assert!(
            js.contains("__error"),
            "must return an error object, never throw"
        );
        assert!(
            !js.contains("__MAX_TEXT__"),
            "placeholder must be fully replaced"
        );
    }

    #[test]
    fn click_and_type_js_escape_the_selector_and_text() {
        // A selector containing a double-quote must be JSON-escaped so it can
        // never break out of the injected script.
        let js = click_js("a[href=\"x\"]");
        assert!(js.contains(r#"document.querySelector("a[href=\"x\"]")"#));

        let ty = type_js("#in", "he said \"hi\"\nline2");
        assert!(ty.contains(r##"querySelector("#in")"##));
        assert!(
            ty.contains(r#""he said \"hi\"\nline2""#),
            "text must be JSON-escaped"
        );
        assert!(
            ty.contains("dispatchEvent"),
            "must notify page frameworks of the edit"
        );
    }

    #[test]
    fn eval_js_wraps_expression_in_try_catch() {
        let js = eval_js("document.title");
        assert!(
            js.contains(r#""document.title""#),
            "source is embedded as a JS string literal"
        );
        assert!(
            js.contains("__error"),
            "raw eval must not throw past the callback"
        );
    }

    #[test]
    fn eval_js_compiles_expression_first_with_eval_fallback() {
        let js = eval_js("1+1");
        assert!(
            js.contains(r#"new Function("return (" + src + "\n)")"#),
            "expression-first construction must be tried before the fallback"
        );
        assert!(
            js.contains("(0, eval)(src)"),
            "fallback must be indirect eval — the only form with completion-value \
             semantics, so multi-statement input answers its last expression \
             (challenge 92aa7d2b: a Function body without return answers undefined)"
        );
        assert!(
            js.contains("fn ? fn() : "),
            "construction and execution must be separate: a runtime throw from the \
             expression path must NOT re-run the source through the fallback"
        );
        assert!(
            js.contains("if (r === undefined) return null;"),
            "undefined results must normalize to null (JSON-serializable)"
        );
    }

    #[test]
    fn eval_js_forces_the_completion_value_json_safe() {
        let js = eval_js("new Date()");
        // The stringify round trip is the app-crash guard: a Date/NaN completion
        // value marshals to NSDate / non-finite NSNumber, and wry hands it to
        // NSJSONSerialization, whose NSInvalidArgumentException kills the whole
        // app (reproduced 2026-07-11, task browser-eval-json-safety). The value
        // must be flattened to JSON IN THE PAGE, before the native bridge.
        assert!(
            js.contains("var s = JSON.stringify(r);"),
            "completion value must go through a JSON.stringify round trip"
        );
        assert!(
            js.contains("return s === undefined ? null : JSON.parse(s);"),
            "stringify-erased values (function/symbol) must answer null, \
             everything else the parsed JSON"
        );
    }

    #[test]
    fn eval_js_escapes_multi_statement_source_with_quotes_and_newlines() {
        let js = eval_js("var x = \"a;b\";\nx + '!'");
        // The whole source — semicolons, quotes, newline — must survive as ONE
        // JSON-escaped string literal so it can never break out of the wrapper.
        assert!(js.contains(r#""var x = \"a;b\";\nx + '!'""#));
        assert!(
            js.contains("__error"),
            "multi-statement eval must not throw past the callback"
        );
    }

    // ── click reliability (task browser-click-reliability) ───────────────────

    /// D1: the whole point of the rewrite — a real pointer sequence, in order,
    /// with `click` LAST. Widgets that ignored `el.click()` listen on
    /// `pointerdown`/`mousedown`, so the order is the contract, not a detail.
    #[test]
    fn click_js_dispatches_pointer_sequence_before_click() {
        let js = click_js("#go");
        let order = [
            "pointerover",
            "pointerenter",
            "mouseover",
            "pointermove",
            "mousemove",
            "pointerdown",
            "mousedown",
            "pointerup",
            "mouseup",
        ];
        let mut at = 0usize;
        for name in order {
            let found = js[at..]
                .find(&format!("\"{name}\""))
                .unwrap_or_else(|| panic!("{name} missing from click_js, or out of order"));
            at += found;
        }
        let click_at = js[at..]
            .find("mouse(\"click\"")
            .expect("the click event itself must come last");
        assert!(click_at > 0, "click must follow the pointer/mouse sequence");
        assert!(
            js.contains("buttons: buttons") && js.contains("pointerdown\", 1"),
            "pointerdown carries buttons:1"
        );
        assert!(js.contains("if (!real) el.click();"), "fallback retained");
    }

    /// D1: an element below the fold has a centre outside the viewport, so the
    /// synthetic coordinates would land on whatever is scrolled into that spot.
    #[test]
    fn click_js_scrolls_into_view() {
        let js = click_js("#go");
        let scroll = js
            .find("scrollIntoView")
            .expect("click_js must scroll the element into view");
        let rect = js
            .find("getBoundingClientRect")
            .expect("click_js must measure the element");
        assert!(
            scroll < rect,
            "scroll BEFORE measuring, or the centre is computed from a stale rect"
        );
    }

    /// D1: the hit element is reported back so an agent can tell "clicked the
    /// wrong node" from "clicked nothing".
    #[test]
    fn click_js_reports_the_hit_element() {
        let js = click_js("#go");
        assert!(js.contains("tag: el.tagName.toLowerCase()"));
        assert!(js.contains("slice(0, 80)"), "text is capped at 80 chars");
    }

    /// D2: both scripts must emit the EXACT miss marker the wait loop retries
    /// on — if these drift, waiting silently stops working.
    #[test]
    fn click_and_type_report_the_retryable_miss_marker() {
        assert!(click_js("#a").contains(SELECTOR_MISS));
        assert!(type_js("#a", "x").contains(SELECTOR_MISS));
        assert!(is_selector_miss(&serde_json::json!({
            "ok": false, "message": SELECTOR_MISS
        })));
        // A real in-page failure must NOT be retried.
        assert!(!is_selector_miss(&serde_json::json!({
            "ok": false, "message": "Permission denied"
        })));
        assert!(!is_selector_miss(&serde_json::json!({ "ok": true })));
    }

    /// D2: the budget runs out and the caller is told what was waited for and
    /// for how long — "no element matched selector" alone cannot distinguish a
    /// wrong selector from a slow page.
    #[tokio::test]
    async fn wait_for_selector_times_out_with_message() {
        let miss = serde_json::json!({ "ok": false, "message": SELECTOR_MISS });
        let calls = std::cell::Cell::new(0u32);
        let out = wait_for_selector(250, "#never", || {
            calls.set(calls.get() + 1);
            let miss = miss.clone();
            async move { Ok(miss) }
        })
        .await
        .expect("a timeout is a result, not an error");

        assert_eq!(out["ok"], false);
        assert_eq!(out["message"], "selector not found within 250ms: #never");
        assert!(calls.get() > 1, "it must actually retry, not answer once");
    }

    /// D2: a hit on a later attempt returns that hit — the whole reason to wait.
    #[tokio::test]
    async fn wait_for_selector_returns_a_late_hit() {
        let calls = std::cell::Cell::new(0u32);
        let out = wait_for_selector(2_000, "#slow", || {
            calls.set(calls.get() + 1);
            let n = calls.get();
            async move {
                Ok(if n < 3 {
                    serde_json::json!({ "ok": false, "message": SELECTOR_MISS })
                } else {
                    serde_json::json!({ "ok": true, "url": "https://x/" })
                })
            }
        })
        .await
        .unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(calls.get(), 3);
    }

    /// D2: `--timeout-ms 0` means "ask once" — no wait, and the raw miss comes
    /// straight back rather than the timeout wording.
    #[tokio::test]
    async fn wait_for_selector_zero_timeout_is_a_single_attempt() {
        let calls = std::cell::Cell::new(0u32);
        let out = wait_for_selector(0, "#x", || {
            calls.set(calls.get() + 1);
            async move { Ok(serde_json::json!({ "ok": false, "message": SELECTOR_MISS })) }
        })
        .await
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(out["message"], SELECTOR_MISS);
    }

    #[test]
    fn resolve_selector_timeout_defaults_and_honours_zero() {
        assert_eq!(resolve_selector_timeout(None), DEFAULT_SELECTOR_TIMEOUT_MS);
        assert_eq!(resolve_selector_timeout(Some(0)), 0);
        assert_eq!(resolve_selector_timeout(Some(750)), 750);
    }

    /// D5: `ping` is two round trips because a synchronous eval cannot count
    /// frames; the read must also stop the counter it armed.
    #[test]
    fn ping_scripts_arm_and_read_a_frame_counter() {
        assert!(PING_ARM_JS.contains("requestAnimationFrame"));
        assert!(PING_READ_JS.contains("cancelAnimationFrame"));
        assert!(PING_READ_JS.contains("rafTicksPer200ms"));
        assert!(PING_READ_JS.contains("document.readyState"));
    }

    /// D5: an agent asking "is this page alive" must be able to read the answer.
    #[test]
    fn ping_result_deserializes_from_the_read_script_shape() {
        let v = serde_json::json!({ "readyState": "complete", "rafTicksPer200ms": 12 });
        let p: BrowserPing = serde_json::from_value(v).unwrap();
        assert_eq!(p.ready_state, "complete");
        assert_eq!(p.raf_ticks_per_200ms, 12);
    }

    #[test]
    fn resolve_bounds_uses_given_rect_and_clamps_negative_size() {
        let (pos, size) = resolve_bounds(Some(Bounds {
            x: 40.0,
            y: 12.0,
            width: 800.0,
            height: -5.0,
        }));
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
        assert_eq!(
            resolve_capture_size(Some(1440.0), Some(900.0)),
            (1440.0, 900.0)
        );
        // below-min clamps up to 1, above-max clamps down to 10000
        assert_eq!(resolve_capture_size(Some(0.0), Some(-4.0)), (1.0, 1.0));
        assert_eq!(resolve_capture_size(Some(99999.0), None), (10000.0, 800.0));
    }
}

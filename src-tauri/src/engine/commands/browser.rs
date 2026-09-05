//! `browser.*` — the in-app browser command family. Parses IPC payloads, calls
//! the `runtime::browser` deep module (which owns all WebView lifecycle +
//! page-tool JS and the per-tab registry), and serializes the results. No JS or
//! WebView logic lives here.
//!
//! # Two callers, one surface
//!
//! - **Agents** drive their OWN tab through the CLI (`conclave browser …`). Every
//!   agent verb (`open`/`goto`/`eval`/`click`/`type`/`snapshot`/`screenshot`/
//!   `close`) auto-scopes to `tabId = <callerAgentId>`. The caller id is derived
//!   server-side from the authenticated caller — the `conclave-cli` client
//!   injects `CONCLAVE_INSTANCE_ID` as the trusted `callerId` field (the same
//!   idiom as `task claim`/`tell`/`memory propose`); an agent CANNOT pass a
//!   `tabId` or `owner` in its payload (anti-spoof). The display name for the
//!   owner label is looked up from the DB by that id.
//! - **The human GUI** drives via the Tauri `invoke` bridge (no `callerId`): it
//!   creates its own tabs (`newTab`), navigates them (`goto { tabId, url }`),
//!   switches the visible tab (`setActive`), positions the overlay
//!   (`setBounds`/`setVisible`), and closes tabs (`close { tabId }`).
//!
//! Every native-touching handler resolves `state.app()` and fails clearly when
//! the app has not finished `.setup()` yet.

use serde::Deserialize;
use serde_json::Value;

use crate::engine::runtime::browser::{self, Bounds, BrowserError};
use crate::engine::runtime::browser_tabs::{BrowserOwner, OwnerKind};
use crate::engine::{AppError, AppState};

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct OpenReq {
    url: String,
    #[serde(default)]
    bounds: Option<Bounds>,
    /// Injected server-side from the caller's `CONCLAVE_INSTANCE_ID` — NEVER a
    /// client-chosen owner (anti-spoof).
    #[serde(default)]
    caller_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GotoReq {
    url: String,
    /// GUI form: the human tab to navigate.
    #[serde(default)]
    tab_id: Option<String>,
    /// Agent form: injected caller id (auto-scopes to the agent's own tab).
    #[serde(default)]
    caller_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SnapshotReq {
    #[serde(default)]
    max_text: Option<i64>,
    #[serde(default)]
    caller_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SelectorReq {
    selector: String,
    /// How long to wait for `selector` to appear, in ms. Absent → the runtime's
    /// 5000 ms default; `0` → a single attempt with no wait.
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    caller_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TypeReq {
    selector: String,
    text: String,
    /// Same contract as [`SelectorReq::timeout_ms`].
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    caller_id: Option<String>,
}

/// `browser.ping` takes nothing but the caller's identity — which tab to probe.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PingReq {
    #[serde(default)]
    caller_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct EvalReq {
    js: String,
    #[serde(default)]
    caller_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ScreenshotReq {
    path: String,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
    #[serde(default)]
    caller_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CloseReq {
    /// GUI form: the specific tab to close. Absent for an agent closing its own.
    #[serde(default)]
    tab_id: Option<String>,
    #[serde(default)]
    caller_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TabIdReq {
    tab_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisibleReq {
    visible: bool,
}

/// Map a runtime-layer browser failure onto the command-layer error type. A
/// bad URL / no-open-tab is the caller's mistake (`Invalid`); a WebView,
/// in-page, or timeout failure is `Internal`.
fn to_app_err(err: BrowserError) -> AppError {
    match err {
        BrowserError::InvalidUrl(_) | BrowserError::NotOpen => AppError::Invalid(err.to_string()),
        BrowserError::Webview(_) | BrowserError::Page(_) | BrowserError::Timeout => {
            AppError::Internal(err.to_string())
        }
    }
}

/// Resolve the Tauri app handle, or a clear error before `.setup()` completes.
fn app_handle(state: &AppState) -> Result<&tauri::AppHandle, AppError> {
    state
        .app()
        .ok_or_else(|| AppError::Internal("browser: app handle not ready".into()))
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, AppError> {
    serde_json::to_value(v).map_err(|e| AppError::Internal(e.to_string()))
}

/// Derive `(tabId, owner)` for an AGENT verb from the trusted caller id. The
/// tab is keyed by the caller's own agent id (D4a: exactly one tab per agent);
/// the owner is always `Agent` with the caller's id and its DB display name.
///
/// This is the anti-spoof boundary: it takes ONLY the server-trusted caller id
/// and name — never an `owner`/`tabId` from the client payload — so an agent
/// can never scope a verb to another agent's tab.
fn resolve_owner(caller_id: &str, name: &str) -> (String, BrowserOwner) {
    (
        caller_id.to_string(),
        BrowserOwner {
            kind: OwnerKind::Agent,
            id: caller_id.to_string(),
            label: name.to_string(),
        },
    )
}

/// The owner descriptor for every human tab (D2/§Interface Contract).
fn human_owner() -> BrowserOwner {
    BrowserOwner {
        kind: OwnerKind::Human,
        id: "human".to_string(),
        label: "You".to_string(),
    }
}

/// Best-effort display name for an agent instance id, via the DB. Falls back to
/// the id itself if the lookup fails — a missing name must never block browsing.
async fn resolve_agent_name(state: &AppState, caller_id: &str) -> String {
    use crate::engine::repo::{agent_definition, workspace_agent};
    let looked_up = async {
        let wa = workspace_agent::get(&state.db, caller_id).await.ok()??;
        let def = agent_definition::get(&state.db, &wa.agent_def_id)
            .await
            .ok()??;
        Some(def.name)
    }
    .await;
    looked_up.unwrap_or_else(|| caller_id.to_string())
}

/// Resolve the agent tab `(tabId, owner)` for a caller id, or a clear error when
/// the caller identity is absent (the verb was run outside a spawned agent).
async fn agent_tab(
    state: &AppState,
    caller_id: Option<&str>,
) -> Result<(String, BrowserOwner), AppError> {
    let caller_id = caller_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::Invalid(
            "browser: caller identity missing — this verb is only available inside a spawned agent"
                .into(),
        )
        })?;
    let name = resolve_agent_name(state, caller_id).await;
    Ok(resolve_owner(caller_id, &name))
}

// ── Agent verbs (auto-scoped to the caller's own tab) ────────────────────────

pub async fn open(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<OpenReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    let (tab_id, owner) = agent_tab(state, req.caller_id.as_deref()).await?;
    to_value(browser::navigate(app, &tab_id, owner, &req.url, req.bounds).map_err(to_app_err)?)
}

/// `browser.goto` serves BOTH callers: an agent navigating its own tab (caller
/// id injected) or the human navigating one of its tabs (`tabId`). Caller id, if
/// present, wins — an agent can never steer another tab by supplying `tabId`.
pub async fn goto(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<GotoReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    if req
        .caller_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        let (tab_id, owner) = agent_tab(state, req.caller_id.as_deref()).await?;
        return to_value(
            browser::navigate(app, &tab_id, owner, &req.url, None).map_err(to_app_err)?,
        );
    }
    match req.tab_id.as_deref() {
        Some(tab_id) => to_value(
            browser::navigate(app, tab_id, human_owner(), &req.url, None).map_err(to_app_err)?,
        ),
        None => Err(AppError::Invalid(
            "browser goto: needs a tabId (human tab) or a caller identity (agent)".into(),
        )),
    }
}

pub async fn snapshot(state: &AppState, payload: Value) -> Result<Value, AppError> {
    // A null/absent payload is valid for a bare `browser snapshot`; use defaults.
    let req = if payload.is_null() {
        SnapshotReq::default()
    } else {
        serde_json::from_value::<SnapshotReq>(payload)
            .map_err(|e| AppError::Invalid(e.to_string()))?
    };
    let app = app_handle(state)?;
    let (tab_id, _owner) = agent_tab(state, req.caller_id.as_deref()).await?;
    to_value(
        browser::snapshot(app, &tab_id, req.max_text)
            .await
            .map_err(to_app_err)?,
    )
}

/// `browser.ping` — "is this page alive": `readyState` plus a 200 ms animation
/// frame count. An agent that gets `rafTicksPer200ms: 0` is looking at a webview
/// that is not painting, which is the real reason a click "did nothing"; the
/// agent can say so instead of escaping to Chrome. Payload is caller-id only.
pub async fn ping(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = if payload.is_null() {
        PingReq::default()
    } else {
        serde_json::from_value::<PingReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?
    };
    let app = app_handle(state)?;
    let (tab_id, _owner) = agent_tab(state, req.caller_id.as_deref()).await?;
    to_value(browser::ping(app, &tab_id).await.map_err(to_app_err)?)
}

pub async fn click(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<SelectorReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    let (tab_id, _owner) = agent_tab(state, req.caller_id.as_deref()).await?;
    to_value(
        browser::click(app, &tab_id, &req.selector, req.timeout_ms)
            .await
            .map_err(to_app_err)?,
    )
}

pub async fn type_text(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<TypeReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    let (tab_id, _owner) = agent_tab(state, req.caller_id.as_deref()).await?;
    to_value(
        browser::type_text(app, &tab_id, &req.selector, &req.text, req.timeout_ms)
            .await
            .map_err(to_app_err)?,
    )
}

/// `browser.eval { js }` — a DELIBERATE debugging escape hatch (plan §CLI
/// Contract / §Risks). It runs agent-supplied JS in the AGENT's OWN page and is
/// safe under this app's trust model: the only caller is the same local user's
/// agent driving its own tab over the same-user UDS surface — it is never
/// exposed over a network or plugin passthrough. The JS is wrapped
/// exception-safe and JSON-forced on the runtime side; the result is JSON.
pub async fn eval(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<EvalReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    let (tab_id, _owner) = agent_tab(state, req.caller_id.as_deref()).await?;
    // eval_json already returns a JSON Value — pass it through verbatim.
    browser::eval_json(app, &tab_id, &req.js)
        .await
        .map_err(to_app_err)
}

pub async fn screenshot(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<ScreenshotReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    if req.path.trim().is_empty() {
        return Err(AppError::Invalid(
            "browser screenshot: path is required".into(),
        ));
    }
    let app = app_handle(state)?;
    let (tab_id, _owner) = agent_tab(state, req.caller_id.as_deref()).await?;
    to_value(
        browser::screenshot(app, &tab_id, &req.path, req.width, req.height)
            .await
            .map_err(to_app_err)?,
    )
}

// ── Shared: close serves both an agent (own tab) and the human (`tabId`) ──────

pub async fn close(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = if payload.is_null() {
        CloseReq::default()
    } else {
        serde_json::from_value::<CloseReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?
    };
    let app = app_handle(state)?;
    // Caller id wins (an agent closes its OWN tab); else the human closes the
    // named tab.
    if let Some(caller_id) = req
        .caller_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return to_value(browser::close_tab(app, caller_id).map_err(to_app_err)?);
    }
    match req.tab_id.as_deref() {
        Some(tab_id) => to_value(browser::close_tab(app, tab_id).map_err(to_app_err)?),
        None => Err(AppError::Invalid(
            "browser close: needs a tabId (human) or a caller identity (agent)".into(),
        )),
    }
}

// ── Read + GUI verbs ─────────────────────────────────────────────────────────

/// The full tab list + active pointer. Read-only — no app handle needed (the
/// registry is valid even before `.setup()`), and it NEVER probes a native URL.
pub async fn status(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    to_value(browser::state())
}

/// Create a new human tab; returns its `tabId`. Registry-only (its webview is
/// created on the first navigation), so no app handle is needed.
pub async fn new_tab(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let tab_id = browser::new_human_tab();
    Ok(serde_json::json!({ "tabId": tab_id }))
}

pub async fn set_active(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<TabIdReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(browser::set_active(app, &req.tab_id).map_err(to_app_err)?)
}

pub async fn set_bounds(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<Bounds>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    browser::set_bounds(app, req).map_err(to_app_err)?;
    Ok(Value::Null)
}

pub async fn set_visible(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<VisibleReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    browser::set_visible(app, req.visible).map_err(to_app_err)?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── resolve_owner: the anti-spoof owner derivation (B3 Step 1) ───────────

    #[test]
    fn resolve_owner_keys_the_tab_by_caller_id_as_agent() {
        let (tab_id, owner) = resolve_owner("agentA", "Guetta");
        assert_eq!(tab_id, "agentA");
        assert_eq!(owner.kind, OwnerKind::Agent);
        assert_eq!(owner.id, "agentA");
        assert_eq!(owner.label, "Guetta");
    }

    /// Anti-spoof: the agent scoping structs deserialize only `callerId` (+ the
    /// verb's own args). A client that stuffs an `owner`/`tabId` into an agent
    /// payload has NO effect — serde drops the unknown field, and `resolve_owner`
    /// only ever sees the server-trusted caller id.
    #[test]
    fn agent_payload_owner_and_tab_id_are_ignored() {
        let payload = json!({
            "url": "https://example.com",
            "callerId": "agentA",
            "owner": { "kind": "human", "id": "spoofed", "label": "Hacker" },
            "tabId": "someone-elses-tab"
        });
        let req: OpenReq = serde_json::from_value(payload).unwrap();
        assert_eq!(req.caller_id.as_deref(), Some("agentA"));
        // Only the caller id reaches resolve_owner — the spoofed fields never do.
        let (tab_id, owner) = resolve_owner(req.caller_id.as_deref().unwrap(), "Guetta");
        assert_eq!(tab_id, "agentA");
        assert_eq!(owner.id, "agentA");
        assert_ne!(owner.label, "Hacker");
    }

    #[test]
    fn human_owner_is_the_you_descriptor() {
        let o = human_owner();
        assert_eq!(o.kind, OwnerKind::Human);
        assert_eq!(o.id, "human");
        assert_eq!(o.label, "You");
    }

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
        let err = set_visible(&state, json!({ "nope": true }))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn screenshot_rejects_blank_path() {
        let state = AppState::for_tests().await;
        let err = screenshot(&state, json!({ "path": "" })).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn set_active_rejects_missing_tab_id() {
        let state = AppState::for_tests().await;
        let err = set_active(&state, json!({})).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    /// An agent verb with no caller identity (run outside a spawned agent, or
    /// before the CLI injection lands) fails clearly rather than silently
    /// colliding on a shared tab.
    #[tokio::test]
    async fn agent_verb_without_caller_id_is_invalid() {
        let state = AppState::for_tests().await;
        let err = super::agent_tab(&state, None).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
        let blank = super::agent_tab(&state, Some("   ")).await.unwrap_err();
        assert!(matches!(blank, AppError::Invalid(_)));
    }

    /// `status` is a pure registry read — it must succeed and return the tab-list
    /// shape (`tabs`/`activeTabId`) even before `.setup()` (no app handle).
    #[tokio::test]
    async fn status_returns_the_tab_list_shape() {
        let state = AppState::for_tests().await;
        let v = status(&state, Value::Null).await.unwrap();
        assert!(
            v.get("tabs").is_some(),
            "status must carry a tabs array: {v}"
        );
        assert!(v["tabs"].is_array());
    }
}

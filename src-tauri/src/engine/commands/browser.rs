//! `browser.*` — the agent-facing in-app browser command family. Parses IPC
//! payloads, calls the `runtime::browser` deep module (which owns all WebView
//! lifecycle + page-tool JS), and serializes the results. No JS or WebView
//! logic lives here.
//!
//! Every tool needs the Tauri `AppHandle` (to reach the shared browser window),
//! so each handler resolves `state.app()` and fails clearly when the app has
//! not finished `.setup()` yet.

use serde::Deserialize;
use serde_json::Value;

use crate::engine::runtime::browser::{self, Bounds, BrowserError};
use crate::engine::{AppError, AppState};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UrlReq {
    url: String,
    #[serde(default)]
    bounds: Option<Bounds>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotReq {
    #[serde(default)]
    max_text: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectorReq {
    selector: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TypeReq {
    selector: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvalReq {
    js: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotReq {
    path: String,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisibleReq {
    visible: bool,
}

/// Map a runtime-layer browser failure onto the command-layer error type. A
/// bad URL / no-open-browser is the caller's mistake (`Invalid`); a WebView,
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

pub async fn open(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<UrlReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(
        browser::open(app, &req.url, req.bounds)
            .await
            .map_err(to_app_err)?,
    )
}

pub async fn goto(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<UrlReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(browser::goto(app, &req.url).await.map_err(to_app_err)?)
}

pub async fn status(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let app = app_handle(state)?;
    to_value(browser::status(app).await.map_err(to_app_err)?)
}

pub async fn snapshot(state: &AppState, payload: Value) -> Result<Value, AppError> {
    // `browser.snapshot` takes an optional { maxText }; a null/absent payload is
    // valid (use the default cap), so tolerate a missing object.
    let req = if payload.is_null() {
        SnapshotReq { max_text: None }
    } else {
        serde_json::from_value::<SnapshotReq>(payload)
            .map_err(|e| AppError::Invalid(e.to_string()))?
    };
    let app = app_handle(state)?;
    to_value(
        browser::snapshot(app, req.max_text)
            .await
            .map_err(to_app_err)?,
    )
}

pub async fn click(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<SelectorReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(
        browser::click(app, &req.selector)
            .await
            .map_err(to_app_err)?,
    )
}

pub async fn type_text(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<TypeReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(
        browser::type_text(app, &req.selector, &req.text)
            .await
            .map_err(to_app_err)?,
    )
}

/// `browser.eval { js }` — a DELIBERATE debugging escape hatch (plan §CLI
/// Contract / §Risks). It runs agent-supplied JS in the page and is safe under
/// this app's trust model: the only caller is the same local user driving their
/// own browser over the same-user UDS surface — it is never exposed over a
/// network or plugin passthrough (`runtime::browser::eval_json` restates this).
/// The JS is wrapped exception-safe on the runtime side; the result is JSON.
pub async fn eval(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<EvalReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    // eval_json already returns a JSON Value — pass it through verbatim.
    browser::eval_json(app, &req.js).await.map_err(to_app_err)
}

pub async fn close(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let app = app_handle(state)?;
    to_value(browser::close(app).await.map_err(to_app_err)?)
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
    to_value(
        browser::screenshot(app, &req.path, req.width, req.height)
            .await
            .map_err(to_app_err)?,
    )
}

pub async fn set_bounds(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<Bounds>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(browser::set_bounds(app, req).await.map_err(to_app_err)?)
}

pub async fn set_visible(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<VisibleReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let app = app_handle(state)?;
    to_value(
        browser::set_visible(app, req.visible)
            .await
            .map_err(to_app_err)?,
    )
}

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
    async fn screenshot_rejects_missing_path() {
        let state = AppState::for_tests().await;
        let err = screenshot(&state, json!({ "width": 800 }))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }
}

use std::sync::atomic::Ordering;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::runtime::ctx_proxy::{MODE_LOG, MODE_OFF, MODE_REWRITE};
use crate::engine::{AppError, AppState};

#[derive(Deserialize)]
struct ModeReq {
    mode: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReportReq {
    since_hours: Option<i64>,
}

pub async fn status(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(status_value(state))
}

pub async fn set_mode(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ModeReq = serde_json::from_value(payload)
        .map_err(|error| AppError::Invalid(format!("proxy.mode: bad payload: {error}")))?;
    let mode = match req.mode.as_str() {
        "off" => MODE_OFF,
        "log" => MODE_LOG,
        "rewrite" => MODE_REWRITE,
        other => {
            return Err(AppError::Invalid(format!(
                "proxy.mode: expected off, log, or rewrite; got '{other}'"
            )))
        }
    };
    state.ctx_proxy.mode.store(mode, Ordering::Release);
    Ok(status_value(state))
}

pub async fn report(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = if payload.is_null() {
        ReportReq::default()
    } else {
        serde_json::from_value::<ReportReq>(payload)
            .map_err(|error| AppError::Invalid(format!("proxy.report: bad payload: {error}")))?
    };
    let since_hours = req.since_hours.unwrap_or(24);
    if since_hours < 0 {
        return Err(AppError::Invalid(
            "proxy.report: sinceHours must be non-negative".into(),
        ));
    }
    Ok(serde_json::to_value(
        crate::engine::repo::proxy_metric::report(&state.db, since_hours).await?,
    )
    .expect("ProxyReport serialization cannot fail"))
}

fn status_value(state: &AppState) -> Value {
    let runtime = &state.ctx_proxy;
    let mode = runtime.mode.load(Ordering::Acquire);
    let conversations = runtime
        .ledger
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .len();
    json!({
        "active": runtime.active_port().is_some(),
        "port": runtime.port,
        "mode": match mode {
            MODE_OFF => "off",
            MODE_REWRITE => "rewrite",
            _ => "log",
        },
        "conversations": conversations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::router;

    #[tokio::test]
    async fn router_status_mode_and_empty_report_roundtrip() {
        let state = AppState::for_tests().await;

        let status = router::dispatch(&state, "proxy.status", Value::Null)
            .await
            .unwrap();
        assert_eq!(status["mode"], "log");
        assert_eq!(status["active"], false);

        let status = router::dispatch(&state, "proxy.mode", json!({ "mode": "rewrite" }))
            .await
            .unwrap();
        assert_eq!(status["mode"], "rewrite");
        assert_eq!(state.ctx_proxy.mode.load(Ordering::Acquire), MODE_REWRITE);

        let report = router::dispatch(&state, "proxy.report", Value::Null)
            .await
            .unwrap();
        assert_eq!(
            report,
            json!({
                "requests": 0,
                "rewritten": 0,
                "bytesSaved": 0,
                "inputTokens": 0,
                "cacheReadTokens": 0,
            })
        );
    }

    #[tokio::test]
    async fn invalid_mode_is_rejected_without_changing_runtime() {
        let state = AppState::for_tests().await;
        let error = router::dispatch(&state, "proxy.mode", json!({ "mode": "nuke" }))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Invalid(_)));
        assert_eq!(state.ctx_proxy.mode.load(Ordering::Acquire), MODE_LOG);
    }
}

use std::sync::atomic::Ordering;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::runtime::ctx_proxy::{MODE_LOG, MODE_OFF, MODE_REWRITE};
use crate::engine::{AppError, AppState};

#[derive(Deserialize)]
struct ModeReq {
    mode: String,
}

#[derive(Deserialize)]
struct ThresholdReq {
    ratio: f32,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReportReq {
    since_hours: Option<i64>,
}

#[derive(Deserialize)]
struct CheckpointReq {
    enabled: bool,
}

#[derive(Deserialize)]
struct CeilingReq {
    tokens: u64,
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

pub async fn set_threshold(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ThresholdReq = serde_json::from_value(payload).map_err(|error| {
        AppError::Invalid(format!("proxy.threshold: bad payload: {error}"))
    })?;
    if !req.ratio.is_finite() || !(0.05..=0.95).contains(&req.ratio) {
        return Err(AppError::Invalid(
            "proxy.threshold: ratio must be a number in [0.05, 0.95]".into(),
        ));
    }
    state
        .ctx_proxy
        .threshold
        .store(req.ratio.to_bits(), Ordering::Release);
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

pub async fn set_checkpoint(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: CheckpointReq = serde_json::from_value(payload)
        .map_err(|e| AppError::Invalid(format!("proxy.checkpoint: bad payload: {e}")))?;
    state.ctx_proxy.checkpoint.store(req.enabled, Ordering::Release);
    Ok(status_value(state))
}

pub async fn set_ceiling(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: CeilingReq = serde_json::from_value(payload)
        .map_err(|e| AppError::Invalid(format!("proxy.ceiling: bad payload: {e}")))?;
    let tokens = u32::try_from(req.tokens)
        .map_err(|_| AppError::Invalid("proxy.ceiling: tokens out of range".into()))?;
    state.ctx_proxy.ceiling.store(tokens, Ordering::Release);
    Ok(status_value(state))
}

pub async fn checkpoint_report(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = if payload.is_null() {
        ReportReq::default()
    } else {
        serde_json::from_value::<ReportReq>(payload)
            .map_err(|e| AppError::Invalid(format!("proxy.checkpointReport: bad payload: {e}")))?
    };
    let since_hours = req.since_hours.unwrap_or(24);
    if since_hours < 0 {
        return Err(AppError::Invalid(
            "proxy.checkpointReport: sinceHours must be non-negative".into(),
        ));
    }
    Ok(serde_json::to_value(
        crate::engine::repo::proxy_checkpoint_metric::report(&state.db, since_hours).await?,
    )
    .expect("CheckpointReport serialization cannot fail"))
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
        "threshold": f32::from_bits(runtime.threshold.load(Ordering::Acquire)),
        "conversations": conversations,
        "checkpoint": runtime.checkpoint.load(Ordering::Acquire),
        "ceiling": runtime.ceiling.load(Ordering::Acquire),
        "checkpointSamplesDropped": runtime.samples_dropped.load(Ordering::Acquire),
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
        // Default threshold is the compile-time high-water, unchanged until set.
        assert!((status["threshold"].as_f64().unwrap() - 0.70).abs() < 1e-6);

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
    async fn checkpoint_toggle_and_ceiling_reflected_in_status() {
        let state = AppState::for_tests().await;
        let status = router::dispatch(&state, "proxy.checkpoint", json!({ "enabled": true })).await.unwrap();
        assert_eq!(status["checkpoint"], true);
        assert!(state.ctx_proxy.checkpoint.load(Ordering::Acquire));

        let status = router::dispatch(&state, "proxy.ceiling", json!({ "tokens": 400_000 })).await.unwrap();
        assert_eq!(status["ceiling"], 400_000);
        assert_eq!(state.ctx_proxy.ceiling.load(Ordering::Acquire), 400_000);

        let report = router::dispatch(&state, "proxy.checkpointReport", Value::Null).await.unwrap();
        assert_eq!(report["samples"], 0);
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

    #[tokio::test]
    async fn valid_threshold_is_reflected_in_status() {
        let state = AppState::for_tests().await;
        let status = router::dispatch(&state, "proxy.threshold", json!({ "ratio": 0.25 }))
            .await
            .unwrap();
        assert!((status["threshold"].as_f64().unwrap() - 0.25).abs() < 1e-6);
        let stored = f32::from_bits(state.ctx_proxy.threshold.load(Ordering::Acquire));
        assert!((stored - 0.25).abs() < 1e-6);
    }

    #[tokio::test]
    async fn out_of_range_threshold_is_rejected_without_changing_runtime() {
        let state = AppState::for_tests().await;
        let before = state.ctx_proxy.threshold.load(Ordering::Acquire);
        for bad in [json!({ "ratio": 0.99 }), json!({ "ratio": 0.01 })] {
            let error = router::dispatch(&state, "proxy.threshold", bad)
                .await
                .unwrap_err();
            assert!(matches!(error, AppError::Invalid(_)));
        }
        assert_eq!(state.ctx_proxy.threshold.load(Ordering::Acquire), before);
    }

    #[tokio::test]
    async fn non_number_threshold_is_rejected() {
        let state = AppState::for_tests().await;
        let before = state.ctx_proxy.threshold.load(Ordering::Acquire);
        let error = router::dispatch(&state, "proxy.threshold", json!({ "ratio": "half" }))
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Invalid(_)));
        assert_eq!(state.ctx_proxy.threshold.load(Ordering::Acquire), before);
    }
}

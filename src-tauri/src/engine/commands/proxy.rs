use std::sync::atomic::Ordering;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::runtime::ctx_proxy::{SummaryArmRequest, SummaryPriceSchedule, SummaryStatus};
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SummaryShadowReq {
    enabled: bool,
    model: Option<String>,
    price_version: Option<String>,
    standard_input_usd_per_mtok: Option<f64>,
    standard_cache_write_usd_per_mtok: Option<f64>,
    standard_cache_read_usd_per_mtok: Option<f64>,
    standard_output_usd_per_mtok: Option<f64>,
    long_context_threshold: Option<u64>,
    long_input_usd_per_mtok: Option<f64>,
    long_cache_write_usd_per_mtok: Option<f64>,
    long_cache_read_usd_per_mtok: Option<f64>,
    long_output_usd_per_mtok: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SummaryReportReq {
    since_hours: Option<i64>,
    campaign_id: Option<String>,
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
    state
        .ctx_proxy
        .checkpoint
        .store(req.enabled, Ordering::Release);
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

pub async fn summary_shadow(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let request: SummaryShadowReq = serde_json::from_value(payload)
        .map_err(|error| AppError::Invalid(format!("proxy.summaryShadow: bad payload: {error}")))?;
    if !request.enabled {
        let has_arm_fields = request.model.is_some()
            || request.price_version.is_some()
            || request.standard_input_usd_per_mtok.is_some()
            || request.standard_cache_write_usd_per_mtok.is_some()
            || request.standard_cache_read_usd_per_mtok.is_some()
            || request.standard_output_usd_per_mtok.is_some()
            || request.long_context_threshold.is_some()
            || request.long_input_usd_per_mtok.is_some()
            || request.long_cache_write_usd_per_mtok.is_some()
            || request.long_cache_read_usd_per_mtok.is_some()
            || request.long_output_usd_per_mtok.is_some();
        if has_arm_fields {
            return Err(AppError::Invalid(
                "proxy.summaryShadow: off accepts no price fields".into(),
            ));
        }
        return Ok(summary_status_value(state.ctx_proxy.disarm_summary()));
    }

    let missing = || AppError::Invalid("proxy.summaryShadow: all arm fields are required".into());
    let arm = SummaryArmRequest {
        model: request.model.ok_or_else(missing)?,
        price: SummaryPriceSchedule {
            price_version: request.price_version.ok_or_else(missing)?,
            standard_input_usd_per_mtok: request.standard_input_usd_per_mtok.ok_or_else(missing)?,
            standard_cache_write_usd_per_mtok: request
                .standard_cache_write_usd_per_mtok
                .ok_or_else(missing)?,
            standard_cache_read_usd_per_mtok: request
                .standard_cache_read_usd_per_mtok
                .ok_or_else(missing)?,
            standard_output_usd_per_mtok: request
                .standard_output_usd_per_mtok
                .ok_or_else(missing)?,
            long_context_threshold: request.long_context_threshold.ok_or_else(missing)?,
            long_input_usd_per_mtok: request.long_input_usd_per_mtok.ok_or_else(missing)?,
            long_cache_write_usd_per_mtok: request
                .long_cache_write_usd_per_mtok
                .ok_or_else(missing)?,
            long_cache_read_usd_per_mtok: request
                .long_cache_read_usd_per_mtok
                .ok_or_else(missing)?,
            long_output_usd_per_mtok: request.long_output_usd_per_mtok.ok_or_else(missing)?,
        },
    };
    let status = state
        .ctx_proxy
        .arm_summary(arm)
        .map_err(|error| AppError::Invalid(format!("proxy.summaryShadow: {}", error.label())))?;
    Ok(summary_status_value(status))
}

pub async fn summary_report(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let request = if payload.is_null() {
        SummaryReportReq::default()
    } else {
        serde_json::from_value::<SummaryReportReq>(payload)
            .map_err(|error| AppError::Invalid(format!("proxy.summaryReport: {error}")))?
    };
    let since_hours = request.since_hours.unwrap_or(24);
    if since_hours < 0 {
        return Err(AppError::Invalid(
            "proxy.summaryReport: sinceHours must be non-negative".into(),
        ));
    }
    let report = crate::engine::repo::proxy_summary_metric::report(
        &state.db,
        since_hours,
        request.campaign_id.as_deref(),
    )
    .await?;
    serde_json::to_value(report)
        .map_err(|error| AppError::Internal(format!("proxy.summaryReport: {error}")))
}

fn summary_status_value(status: SummaryStatus) -> Value {
    let price = status.price.as_ref();
    json!({
        "summaryShadow": status.armed,
        "summaryCampaignId": status.campaign_id,
        "summaryModel": status.model,
        "summaryPriceVersion": price.map(|price| price.price_version.as_str()),
        "summaryStandardInputUsdPerMtok": price.map(|price| price.standard_input_usd_per_mtok),
        "summaryStandardCacheWriteUsdPerMtok": price.map(|price| price.standard_cache_write_usd_per_mtok),
        "summaryStandardCacheReadUsdPerMtok": price.map(|price| price.standard_cache_read_usd_per_mtok),
        "summaryStandardOutputUsdPerMtok": price.map(|price| price.standard_output_usd_per_mtok),
        "summaryLongContextThreshold": price.map(|price| price.long_context_threshold),
        "summaryLongInputUsdPerMtok": price.map(|price| price.long_input_usd_per_mtok),
        "summaryLongCacheWriteUsdPerMtok": price.map(|price| price.long_cache_write_usd_per_mtok),
        "summaryLongCacheReadUsdPerMtok": price.map(|price| price.long_cache_read_usd_per_mtok),
        "summaryLongOutputUsdPerMtok": price.map(|price| price.long_output_usd_per_mtok),
        "summaryTailTargetTokens": status.tail_target_tokens,
        "summaryMaxOutputTokens": status.max_output_tokens,
        "summarySamplesDropped": status.samples_dropped,
        "summaryModelMismatch": status.model_mismatch,
        "summaryInFlight": status.in_flight,
    })
}

fn status_value(state: &AppState) -> Value {
    let runtime = &state.ctx_proxy;
    let mode = runtime.mode.load(Ordering::Acquire);
    let conversations = runtime
        .ledger
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .len();
    let mut value = json!({
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
    });
    let summary = summary_status_value(runtime.summary_status());
    value.as_object_mut().expect("status is an object").extend(
        summary
            .as_object()
            .expect("summary status is an object")
            .clone(),
    );
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::router;

    fn summary_arm_payload() -> Value {
        json!({
            "enabled":true,
            "model":"claude-sonnet-5[1m]",
            "priceVersion":"anthropic-2026-07-12",
            "standardInputUsdPerMtok":3.0,
            "standardCacheWriteUsdPerMtok":3.75,
            "standardCacheReadUsdPerMtok":0.30,
            "standardOutputUsdPerMtok":15.0,
            "longContextThreshold":200000,
            "longInputUsdPerMtok":6.0,
            "longCacheWriteUsdPerMtok":7.5,
            "longCacheReadUsdPerMtok":0.60,
            "longOutputUsdPerMtok":22.5,
        })
    }

    #[tokio::test]
    async fn summary_shadow_is_off_by_default_and_status_is_redaction_safe() {
        let state = AppState::for_tests().await;
        let status = router::dispatch(&state, "proxy.status", Value::Null)
            .await
            .unwrap();
        assert_eq!(status["summaryShadow"], false);
        assert!(status["summaryCampaignId"].is_null());
        assert!(status["summaryModel"].is_null());
        assert!(status["summaryPriceVersion"].is_null());
        assert_eq!(status["summaryTailTargetTokens"], 100_000);
        assert_eq!(status["summaryMaxOutputTokens"], 8_192);
        assert_eq!(status["summarySamplesDropped"], 0);
        assert_eq!(status["summaryModelMismatch"], 0);
        assert_eq!(status["summaryInFlight"], 0);
        let serialized = status.to_string().to_ascii_lowercase();
        for forbidden in ["credential", "authorization", "api_key", "prompt", "source"] {
            assert!(!serialized.contains(forbidden), "leaked key: {forbidden}");
        }
    }

    #[tokio::test]
    async fn summary_shadow_arm_is_atomic_and_off_clears_runtime_config() {
        let state = AppState::for_tests().await;
        let status = router::dispatch(&state, "proxy.summaryShadow", summary_arm_payload())
            .await
            .unwrap();
        assert_eq!(status["summaryShadow"], true);
        assert_eq!(status["summaryModel"], "claude-sonnet-5[1m]");
        assert_eq!(status["summaryPriceVersion"], "anthropic-2026-07-12");
        assert_eq!(status["summaryStandardCacheReadUsdPerMtok"], 0.30);
        let before = state.ctx_proxy.snapshot_summary_campaign().unwrap().1;
        let report = router::dispatch(&state, "proxy.summaryReport", Value::Null)
            .await
            .unwrap();
        assert_eq!(report["totalAdmitted"], 0, "arming alone performs no work");

        let mut invalid = summary_arm_payload();
        invalid["standardCacheReadUsdPerMtok"] = json!(0.0);
        assert!(router::dispatch(&state, "proxy.summaryShadow", invalid)
            .await
            .is_err());
        let after = state.ctx_proxy.snapshot_summary_campaign().unwrap().1;
        assert_eq!(before, after, "rejected arm must preserve prior campaign");

        let status = router::dispatch(&state, "proxy.summaryShadow", json!({"enabled":false}))
            .await
            .unwrap();
        assert_eq!(status["summaryShadow"], false);
        assert!(state.ctx_proxy.snapshot_summary_campaign().is_none());

        let restarted = AppState::for_tests().await;
        assert!(!restarted.ctx_proxy.summary_status().armed);
    }

    #[tokio::test]
    async fn summary_report_maps_repository_output_without_arming() {
        let state = AppState::for_tests().await;
        let report = router::dispatch(
            &state,
            "proxy.summaryReport",
            json!({"sinceHours":12,"campaignId":"campaign-1"}),
        )
        .await
        .unwrap();
        assert_eq!(report["totalAdmitted"], 0);
        assert_eq!(report["measured"], 0);
        assert!(!state.ctx_proxy.summary_status().armed);
    }

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
        let status = router::dispatch(&state, "proxy.checkpoint", json!({ "enabled": true }))
            .await
            .unwrap();
        assert_eq!(status["checkpoint"], true);
        assert!(state.ctx_proxy.checkpoint.load(Ordering::Acquire));

        let status = router::dispatch(&state, "proxy.ceiling", json!({ "tokens": 400_000 }))
            .await
            .unwrap();
        assert_eq!(status["ceiling"], 400_000);
        assert_eq!(state.ctx_proxy.ceiling.load(Ordering::Acquire), 400_000);

        let report = router::dispatch(&state, "proxy.checkpointReport", Value::Null)
            .await
            .unwrap();
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

use std::sync::atomic::Ordering;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::runtime::ctx_proxy::{
    QualityArmRequest, QualityStatus, SummaryArmRequest, SummaryPriceSchedule, SummaryStatus,
};
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualityShadowReq {
    enabled: bool,
    h1_campaign_id: Option<String>,
    evaluator_model: Option<String>,
    rubric_version: Option<String>,
    max_cases: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualityFixturesReq {
    manifest: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualityReportReq {
    since_hours: Option<i64>,
    campaign_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QualityAuditReq {
    enabled: bool,
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
    let req: ThresholdReq = serde_json::from_value(payload)
        .map_err(|error| AppError::Invalid(format!("proxy.threshold: bad payload: {error}")))?;
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

pub async fn quality_shadow(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let request: QualityShadowReq = serde_json::from_value(payload)
        .map_err(|error| AppError::Invalid(format!("proxy.qualityShadow: bad payload: {error}")))?;
    if !request.enabled {
        if request.h1_campaign_id.is_some()
            || request.evaluator_model.is_some()
            || request.rubric_version.is_some()
            || request.max_cases.is_some()
        {
            return Err(AppError::Invalid(
                "proxy.qualityShadow: off accepts no arm fields".into(),
            ));
        }
        return Ok(quality_status_value(state.ctx_proxy.disarm_quality()));
    }

    let missing = || AppError::Invalid("proxy.qualityShadow: all arm fields are required".into());
    let h1_campaign_id = request.h1_campaign_id.ok_or_else(missing)?;
    let evaluator_model = request.evaluator_model.ok_or_else(missing)?;
    let rubric_version = request.rubric_version.ok_or_else(missing)?;
    let max_cases = request.max_cases.ok_or_else(missing)?;
    let summary_status = state.ctx_proxy.summary_status();
    let h1_report =
        crate::engine::repo::proxy_summary_metric::report_campaign(&state.db, &h1_campaign_id)
            .await?;
    let task_model = validate_h1_quality_arm(&summary_status, &h1_report, &h1_campaign_id)?;
    let status = state
        .ctx_proxy
        .arm_quality(QualityArmRequest {
            h1_campaign_id,
            evaluator_model,
            task_model,
            rubric_version,
            max_cases,
        })
        .map_err(|error| AppError::Invalid(format!("proxy.qualityShadow: {}", error.label())))?;
    // Re-arm replaces the campaign; no raw audit material from the prior
    // campaign may survive a successful state transition.
    state.ctx_proxy.quality_audit.clear();
    Ok(quality_status_value(status))
}

fn validate_h1_quality_arm(
    status: &SummaryStatus,
    report: &crate::engine::repo::proxy_summary_metric::SummaryReport,
    requested_campaign_id: &str,
) -> Result<String, AppError> {
    use crate::engine::repo::proxy_summary_metric::{h1_gate, H1Gate};

    if !status.armed || status.campaign_id.as_deref() != Some(requested_campaign_id) {
        return Err(AppError::Invalid(
            "proxy.qualityShadow: linked H1 campaign is not currently armed".into(),
        ));
    }
    let task_model = status.model.clone().ok_or_else(|| {
        AppError::Invalid("proxy.qualityShadow: linked H1 model is unavailable".into())
    })?;
    if report.by_price_version_and_model.is_empty()
        || report
            .by_price_version_and_model
            .iter()
            .any(|group| group.model != task_model)
    {
        return Err(AppError::Invalid(
            "proxy.qualityShadow: linked H1 rows are not model-identical".into(),
        ));
    }
    if h1_gate(report) != H1Gate::Pass {
        return Err(AppError::Invalid(format!(
            "proxy.qualityShadow: linked H1 gate is {}",
            match h1_gate(report) {
                H1Gate::Pass => "pass",
                H1Gate::Inconclusive => "inconclusive",
                H1Gate::Fail => "fail",
            }
        )));
    }
    Ok(task_model)
}

pub async fn quality_fixtures(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let request: QualityFixturesReq = serde_json::from_value(payload)
        .map_err(|error| AppError::Invalid(format!("proxy.qualityFixtures: {error}")))?;
    if request.manifest != "h2-adversarial-v1" {
        return Err(AppError::Invalid(
            "proxy.qualityFixtures: unknown manifest".into(),
        ));
    }
    if !state.ctx_proxy.quality_status().armed {
        return Err(AppError::Invalid(
            "proxy.qualityFixtures: H2 is not armed".into(),
        ));
    }
    let fixtures = crate::engine::runtime::quality_fixtures::load_h2_adversarial_manifest()
        .map_err(|error| AppError::Internal(format!("proxy.qualityFixtures: {error}")))?;
    let ids: Vec<String> = fixtures
        .into_iter()
        .map(|fixture| fixture.case.id)
        .collect();
    state.ctx_proxy.enqueue_quality_fixtures(&ids);
    Ok(quality_status_value(state.ctx_proxy.quality_status()))
}

pub async fn quality_report(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let request = if payload.is_null() {
        QualityReportReq::default()
    } else {
        serde_json::from_value::<QualityReportReq>(payload)
            .map_err(|error| AppError::Invalid(format!("proxy.qualityReport: {error}")))?
    };
    if request.since_hours.is_some_and(|hours| hours < 0) {
        return Err(AppError::Invalid(
            "proxy.qualityReport: sinceHours must be non-negative".into(),
        ));
    }
    if request
        .campaign_id
        .as_deref()
        .is_some_and(|campaign_id| campaign_id.trim().is_empty())
    {
        return Err(AppError::Invalid(
            "proxy.qualityReport: campaignId must not be empty".into(),
        ));
    }
    let report = crate::engine::repo::proxy_quality_metric::report(
        &state.db,
        request.since_hours,
        request.campaign_id.as_deref(),
    )
    .await?;
    let (h1_armed, h1_gate) = linked_h1_gate(state, request.campaign_id.as_deref()).await;
    let bars = crate::engine::repo::proxy_quality_metric::evaluate_go(&report, h1_armed, h1_gate);
    let mut value = serde_json::to_value(report)
        .map_err(|error| AppError::Internal(format!("proxy.qualityReport: {error}")))?;
    value["goBars"] = serde_json::to_value(bars)
        .map_err(|error| AppError::Internal(format!("proxy.qualityReport: {error}")))?;
    Ok(value)
}

async fn linked_h1_gate(
    state: &AppState,
    quality_campaign_id: Option<&str>,
) -> (bool, crate::engine::repo::proxy_summary_metric::H1Gate) {
    use crate::engine::repo::proxy_summary_metric::{h1_gate, report_campaign, H1Gate};
    let Some(quality_campaign_id) = quality_campaign_id else {
        return (false, H1Gate::Inconclusive);
    };
    let Some((_, quality)) = state.ctx_proxy.snapshot_quality_campaign() else {
        return (false, H1Gate::Inconclusive);
    };
    if quality.quality_campaign_id != quality_campaign_id {
        return (false, H1Gate::Inconclusive);
    }
    let Some((_, summary)) = state.ctx_proxy.snapshot_summary_campaign() else {
        return (false, H1Gate::Inconclusive);
    };
    if summary.campaign_id != quality.h1_campaign_id {
        return (false, H1Gate::Inconclusive);
    }
    match report_campaign(&state.db, &summary.campaign_id).await {
        Ok(report) => (true, h1_gate(&report)),
        Err(_) => (true, H1Gate::Fail),
    }
}

pub async fn quality_audit(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let request: QualityAuditReq = serde_json::from_value(payload)
        .map_err(|error| AppError::Invalid(format!("proxy.qualityAudit: {error}")))?;
    if !request.enabled {
        if request.campaign_id.is_some() {
            return Err(AppError::Invalid(
                "proxy.qualityAudit: stop accepts no campaign id".into(),
            ));
        }
        state.ctx_proxy.quality_audit.clear();
        return Ok(audit_status_value(state));
    }
    let campaign_id = request
        .campaign_id
        .ok_or_else(|| AppError::Invalid("proxy.qualityAudit: campaignId is required".into()))?;
    if campaign_id.trim().is_empty() {
        return Err(AppError::Invalid(
            "proxy.qualityAudit: campaignId must not be empty".into(),
        ));
    }
    let current = state
        .ctx_proxy
        .snapshot_quality_campaign()
        .ok_or_else(|| AppError::Invalid("proxy.qualityAudit: H2 is not armed".into()))?;
    if current.1.quality_campaign_id != campaign_id {
        return Err(AppError::Invalid(
            "proxy.qualityAudit: campaign is not the armed H2 campaign".into(),
        ));
    }
    let started = state
        .ctx_proxy
        .quality_audit
        .start(state.db.clone(), campaign_id)
        .await
        .map_err(|error| AppError::Internal(format!("proxy.qualityAudit: {error}")))?;
    let mut value = audit_status_value(state);
    value["auditUrl"] = json!(started.url);
    Ok(value)
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

fn quality_status_value(status: QualityStatus) -> Value {
    let preflight_state = if !status.armed {
        "off"
    } else if status.preflight_pending {
        "pending"
    } else {
        "verified"
    };
    json!({
        "qualityShadow": status.armed,
        "qualityCampaignId": status.quality_campaign_id,
        "qualityH1CampaignId": status.h1_campaign_id,
        "qualityEvaluatorModel": status.evaluator_model,
        "qualityRubricVersion": status.rubric_version,
        "qualityMaxCases": status.max_cases,
        "qualityRemainingCases": status.remaining_cases,
        "qualityPreflightState": preflight_state,
        "qualityFixtureQueueLength": status.fixture_queue_len,
        "qualitySamplesDropped": status.samples_dropped,
        "qualityH1Blocked": status.h1_blocked,
        "qualityModelMismatch": status.model_mismatch,
        "qualityCredentialMismatch": status.credential_mismatch,
        "qualityInFlight": status.in_flight,
    })
}

fn audit_status_value(state: &AppState) -> Value {
    let status = state.ctx_proxy.quality_audit.status();
    json!({
        "qualityAuditActive": status.active,
        "qualityAuditSelected": status.selected,
        "qualityAuditSubmitted": status.submitted,
        "qualityAuditExpiresInSeconds": status.expires_in_seconds,
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
    let quality = quality_status_value(runtime.quality_status());
    value.as_object_mut().expect("status is an object").extend(
        quality
            .as_object()
            .expect("quality status is an object")
            .clone(),
    );
    let audit = audit_status_value(state);
    value.as_object_mut().expect("status is an object").extend(
        audit
            .as_object()
            .expect("audit status is an object")
            .clone(),
    );
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::proxy_summary_metric::{
        ConversationPassCount, SummaryGroupCount, SummaryReport,
    };
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

    fn passing_h1_report(model: &str) -> SummaryReport {
        SummaryReport {
            total_admitted: 40,
            measured: 30,
            disarmed: 0,
            below_ceiling: 5,
            tail_boundary_failures: 0,
            no_candidate: 3,
            count_failures: 1,
            generation_failures: 0,
            projection_rejected: 1,
            metric_invalid: 0,
            distinct_conversations: 10,
            failure_rate: 1.0 / 31.0,
            band_250k_350k: 10,
            pct_meets_low_water: 0.93,
            pct_meets_two_turn: 0.86,
            q_h_min: 0.4,
            q_h_median: 0.6,
            q_h_max: 0.9,
            q_h_avg: 0.6,
            n_h_min: 0.5,
            n_h_median: 1.0,
            n_h_max: 1.9,
            n_h_avg: 1.1,
            max_plateau_turns: 3,
            gen_input_tokens_total: 1_000_000,
            gen_cache_creation_tokens_total: 10_000,
            gen_cache_read_tokens_total: 50_000,
            gen_output_tokens_total: 20_000,
            by_price_version_and_model: vec![SummaryGroupCount {
                price_version: "price-v1".into(),
                model: model.into(),
                count: 40,
            }],
            conversation_pass_counts: (0..10)
                .map(|index| ConversationPassCount {
                    conversation_hash: format!("conversation-{index}"),
                    measured_candidates: 3,
                    passing_candidates: 2,
                })
                .collect(),
        }
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
        assert_eq!(status["qualityCredentialMismatch"], 0);
        for forbidden in [
            "credentialidentity",
            "authorization",
            "api_key",
            "prompt",
            "source",
        ] {
            assert!(!serialized.contains(forbidden), "leaked key: {forbidden}");
        }
    }

    #[tokio::test]
    async fn quality_status_is_off_by_default_and_arm_requires_current_passing_h1() {
        let state = AppState::for_tests().await;
        let status = router::dispatch(&state, "proxy.status", Value::Null)
            .await
            .unwrap();
        assert_eq!(status["qualityShadow"], false);
        assert_eq!(status["qualityPreflightState"], "off");
        assert!(status["qualityCampaignId"].is_null());
        assert_eq!(status["qualityRemainingCases"], 0);
        assert_eq!(status["qualityFixtureQueueLength"], 0);
        assert_eq!(status["qualityAuditActive"], false);

        let payload = json!({
            "enabled":true,
            "h1CampaignId":"missing-h1",
            "evaluatorModel":"evaluator-model",
            "rubricVersion":"hybrid-quality-rubric-v1",
            "maxCases":100,
        });
        assert!(router::dispatch(&state, "proxy.qualityShadow", payload)
            .await
            .is_err());
        assert!(!state.ctx_proxy.quality_status().armed);
    }

    #[test]
    fn quality_status_preflight_state_is_exactly_off_pending_or_verified() {
        let status = |armed, preflight_pending| QualityStatus {
            armed,
            quality_campaign_id: armed.then(|| "quality-campaign".into()),
            h1_campaign_id: armed.then(|| "h1-campaign".into()),
            evaluator_model: armed.then(|| "evaluator-model".into()),
            rubric_version: armed.then(|| "hybrid-quality-rubric-v1".into()),
            max_cases: armed.then_some(100),
            remaining_cases: u64::from(armed) * 100,
            preflight_pending,
            fixture_queue_len: 0,
            samples_dropped: 0,
            h1_blocked: 0,
            model_mismatch: 0,
            credential_mismatch: 0,
            in_flight: 0,
        };

        assert_eq!(
            quality_status_value(status(false, true))["qualityPreflightState"],
            "off"
        );
        assert_eq!(
            quality_status_value(status(true, true))["qualityPreflightState"],
            "pending"
        );
        assert_eq!(
            quality_status_value(status(true, false))["qualityPreflightState"],
            "verified"
        );
    }

    #[tokio::test]
    async fn quality_handlers_reject_empty_campaign_filters_at_ipc_boundary() {
        let state = AppState::for_tests().await;
        for campaign_id in ["", " \t"] {
            let report = router::dispatch(
                &state,
                "proxy.qualityReport",
                json!({"campaignId": campaign_id}),
            )
            .await
            .unwrap_err();
            assert!(matches!(report, AppError::Invalid(_)));

            let audit = router::dispatch(
                &state,
                "proxy.qualityAudit",
                json!({"enabled": true, "campaignId": campaign_id}),
            )
            .await
            .unwrap_err();
            assert!(matches!(audit, AppError::Invalid(_)));
        }
    }

    #[test]
    fn quality_arm_h1_validation_pins_pass_campaign_and_model_identity() {
        let status = SummaryStatus {
            armed: true,
            campaign_id: Some("h1-campaign".into()),
            model: Some("task-model".into()),
            price: None,
            tail_target_tokens: 100_000,
            max_output_tokens: 8_192,
            samples_dropped: 0,
            model_mismatch: 0,
            in_flight: 0,
        };
        let report = passing_h1_report("task-model");
        assert_eq!(
            validate_h1_quality_arm(&status, &report, "h1-campaign").unwrap(),
            "task-model"
        );
        let mut wrong_model = report.clone();
        wrong_model.by_price_version_and_model[0].model = "other-model".into();
        assert!(validate_h1_quality_arm(&status, &wrong_model, "h1-campaign").is_err());
        let mut inconclusive = report;
        inconclusive.measured = 29;
        assert!(validate_h1_quality_arm(&status, &inconclusive, "h1-campaign").is_err());
    }

    #[tokio::test]
    async fn fixture_enqueue_and_audit_start_are_zero_call_control_operations() {
        let state = AppState::for_tests().await;
        let armed = state
            .ctx_proxy
            .arm_quality(QualityArmRequest {
                h1_campaign_id: "h1-campaign".into(),
                evaluator_model: "evaluator-model".into(),
                task_model: "task-model".into(),
                rubric_version: "hybrid-quality-rubric-v1".into(),
                max_cases: 100,
            })
            .unwrap();
        let campaign_id = armed.quality_campaign_id.unwrap();
        let status = router::dispatch(
            &state,
            "proxy.qualityFixtures",
            json!({"manifest":"h2-adversarial-v1"}),
        )
        .await
        .unwrap();
        assert_eq!(status["qualityFixtureQueueLength"], 70);
        assert_eq!(status["qualityRemainingCases"], 100);
        assert_eq!(status["qualityInFlight"], 0);

        let audit = router::dispatch(
            &state,
            "proxy.qualityAudit",
            json!({"enabled":true,"campaignId":campaign_id}),
        )
        .await
        .unwrap();
        assert_eq!(audit["qualityAuditActive"], true);
        assert!(audit["auditUrl"]
            .as_str()
            .unwrap()
            .starts_with("http://127.0.0.1:"));
        let stopped = router::dispatch(&state, "proxy.qualityAudit", json!({"enabled":false}))
            .await
            .unwrap();
        assert_eq!(stopped["qualityAuditActive"], false);
    }

    #[tokio::test]
    async fn quality_report_is_read_only_and_off_clears_audit_and_queue() {
        use crate::engine::runtime::quality::{JudgeScores, QualityTag};

        let state = AppState::for_tests().await;
        let armed = state
            .ctx_proxy
            .arm_quality(QualityArmRequest {
                h1_campaign_id: "h1-campaign".into(),
                evaluator_model: "evaluator-model".into(),
                task_model: "task-model".into(),
                rubric_version: "hybrid-quality-rubric-v1".into(),
                max_cases: 100,
            })
            .unwrap();
        let campaign_id = armed.quality_campaign_id.unwrap();
        router::dispatch(
            &state,
            "proxy.qualityFixtures",
            json!({"manifest":"h2-adversarial-v1"}),
        )
        .await
        .unwrap();
        router::dispatch(
            &state,
            "proxy.qualityAudit",
            json!({"enabled":true,"campaignId":campaign_id.clone()}),
        )
        .await
        .unwrap();
        let scores = JudgeScores {
            correct: true,
            constraint_adherent: true,
            next_action_match: true,
        };
        let bundle = crate::engine::runtime::quality_audit::FixtureAuditBundle::completed(
            uuid::Uuid::new_v4().to_string(),
            "exact-error-01",
            vec![QualityTag::ExactError],
            "[]".into(),
            "summary".into(),
            &[],
            "original".into(),
            "projected".into(),
            "{}".into(),
            scores,
            scores,
            true,
        )
        .unwrap();
        assert!(state
            .ctx_proxy
            .quality_audit
            .offer_fixture(&campaign_id, bundle));
        assert_eq!(state.ctx_proxy.quality_audit.status().selected, 1);
        let report = router::dispatch(
            &state,
            "proxy.qualityReport",
            json!({"campaignId":campaign_id}),
        )
        .await
        .unwrap();
        assert_eq!(report["terminalCases"], 0);
        assert_eq!(report["goBars"]["go"], false);
        assert!(state.ctx_proxy.quality_status().armed);

        let off = router::dispatch(&state, "proxy.qualityShadow", json!({"enabled":false}))
            .await
            .unwrap();
        assert_eq!(off["qualityShadow"], false);
        assert_eq!(off["qualityFixtureQueueLength"], 0);
        let audit = state.ctx_proxy.quality_audit.status();
        assert!(!audit.active);
        assert_eq!(audit.selected, 0);
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

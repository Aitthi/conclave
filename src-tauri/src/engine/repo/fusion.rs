//! Fusion repository — persistence for a context-fusion run (M4.3).
//!
//! A `fusion_run` records one orchestrated pipeline: a prompt fanned out to a
//! panel of agents, a judge's structured analysis, and a synthesized final
//! answer. Each panel member's reply is a `fusion_panel_response` row.
//!
//! # honesty seam (M4.3)
//!
//! A panel member that cannot run (provider not configured / no key / HTTP
//! error) is persisted with `status = "error"` and the honest error string in
//! `answer` — never a fabricated answer. `judge_analysis` stores the STRING form
//! of the parsed JSON (an honest `{ "raw", "parseError": true }` fallback when
//! the judge returns non-JSON). The M4.3 panel is DERIVED from the workspace's
//! chat agents; explicit `fusion_config` / `fusion_panel_member` selection is
//! M4.4.
//!
//! # chain-builder usage
//!
//! All queries use chain-builder (single-table SELECT / INSERT / UPDATE),
//! mirroring `session.rs` and `workspace_agent.rs`.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite, Value as Bind};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Row structs ───────────────────────────────────────────────────────────────

/// Decoded row from the `fusion_run` table.
///
/// `sqlx::FromRow` maps snake_case columns to snake_case fields.
/// `serde(rename_all = "camelCase")` emits camelCase JSON matching the
/// `FusionRun` interface in `src/ipc/types.ts`. Optional columns use
/// `skip_serializing_if` so an absent value is a missing key (matching the
/// `field?: T` TS types), never `null`.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FusionRunRow {
    pub id: String,
    pub session_id: String, // → "sessionId"
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge_analysis: Option<String>, // → "judgeAnalysis"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesized: Option<String>,
    pub created_at: String, // → "createdAt"
}

/// Decoded row from the `fusion_panel_response` table.
///
/// Matches the `FusionPanelResponse` interface in `src/ipc/types.ts`.
/// `status` is one of `"running" | "done" | "error"` (the schema CHECK enforces
/// this). `answer` holds the panel member's reply OR (on `"error"`) the honest
/// error message.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FusionPanelResponseRow {
    pub id: String,
    pub fusion_run_id: String, // → "fusionRunId"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>, // → "instanceId"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    pub status: String,
    pub created_at: String, // → "createdAt"
}

// ── Column lists ───────────────────────────────────────────────────────────────

const RUN_COLS: [&str; 6] = [
    "id",
    "session_id",
    "prompt",
    "judge_analysis",
    "synthesized",
    "created_at",
];

// Used by `list_responses` (tests + M4.4 read path), not yet by production.
#[allow(dead_code)]
const RESP_COLS: [&str; 6] = [
    "id",
    "fusion_run_id",
    "instance_id",
    "answer",
    "status",
    "created_at",
];

// ── CRUD ──────────────────────────────────────────────────────────────────────

/// Create a `fusion_run` row (fresh: `judge_analysis` / `synthesized` NULL) and
/// return the constructed row. Generates a UUID v4 `id` and ISO-8601 UTC
/// `created_at`.
pub async fn create_run(
    pool: &SqlitePool,
    session_id: &str,
    prompt: &str,
) -> sqlx::Result<FusionRunRow> {
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();

    QueryBuilder::<Sqlite>::table("fusion_run")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("session_id", Bind::Text(session_id.to_owned())),
            ("prompt", Bind::Text(prompt.to_owned())),
            ("judge_analysis", Bind::Null),
            ("synthesized", Bind::Null),
            ("created_at", Bind::Text(created_at.clone())),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(FusionRunRow {
        id,
        session_id: session_id.to_owned(),
        prompt: prompt.to_owned(),
        judge_analysis: None,
        synthesized: None,
        created_at,
    })
}

/// Persist the judge's analysis (the STRING form of the parsed JSON) onto a run.
pub async fn set_judge_analysis(
    pool: &SqlitePool,
    run_id: &str,
    analysis_json: &str,
) -> sqlx::Result<()> {
    QueryBuilder::<Sqlite>::table("fusion_run")
        .update([("judge_analysis", Bind::Text(analysis_json.to_owned()))])
        .where_eq("id", run_id)
        .execute(pool)
        .await
        .map_err(cb_err)?;
    Ok(())
}

/// Persist the synthesized final answer onto a run.
pub async fn set_synthesized(pool: &SqlitePool, run_id: &str, text: &str) -> sqlx::Result<()> {
    QueryBuilder::<Sqlite>::table("fusion_run")
        .update([("synthesized", Bind::Text(text.to_owned()))])
        .where_eq("id", run_id)
        .execute(pool)
        .await
        .map_err(cb_err)?;
    Ok(())
}

/// Insert a panel response and return the constructed row.
///
/// `answer` is `Some(text)` for a `"done"` reply or `Some(error_message)` for an
/// `"error"` row (the honest failure string — never a fabricated answer).
pub async fn create_panel_response(
    pool: &SqlitePool,
    fusion_run_id: &str,
    instance_id: &str,
    answer: Option<&str>,
    status: &str,
) -> sqlx::Result<FusionPanelResponseRow> {
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();

    QueryBuilder::<Sqlite>::table("fusion_panel_response")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("fusion_run_id", Bind::Text(fusion_run_id.to_owned())),
            ("instance_id", Bind::Text(instance_id.to_owned())),
            (
                "answer",
                answer
                    .map(|s| Bind::Text(s.to_owned()))
                    .unwrap_or(Bind::Null),
            ),
            ("status", Bind::Text(status.to_owned())),
            ("created_at", Bind::Text(created_at.clone())),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(FusionPanelResponseRow {
        id,
        fusion_run_id: fusion_run_id.to_owned(),
        instance_id: Some(instance_id.to_owned()),
        answer: answer.map(|s| s.to_owned()),
        status: status.to_owned(),
        created_at,
    })
}

/// Fetch a fusion run by its primary-key `id`, or `None` if no such run.
pub async fn get_run(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<FusionRunRow>> {
    QueryBuilder::<Sqlite>::table("fusion_run")
        .select(RUN_COLS)
        .where_eq("id", id)
        .fetch_optional::<FusionRunRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// List every panel response for a run, oldest-first (`created_at`, `id` tie-break).
///
/// `#[allow(dead_code)]`: no production caller yet — used by tests and the M4.4
/// fusion-result read path (the run pipeline persists responses but doesn't read
/// them back).
#[allow(dead_code)]
pub async fn list_responses(
    pool: &SqlitePool,
    fusion_run_id: &str,
) -> sqlx::Result<Vec<FusionPanelResponseRow>> {
    QueryBuilder::<Sqlite>::table("fusion_panel_response")
        .select(RESP_COLS)
        .where_eq("fusion_run_id", fusion_run_id)
        .order_by("created_at", Order::Asc)
        .order_by("id", Order::Asc)
        .fetch_all::<FusionPanelResponseRow, _>(pool)
        .await
        .map_err(cb_err)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        db::connect_in_memory,
        repo::{
            agent_definition::{self, AgentDefinitionInput},
            session, workspace, workspace_agent,
        },
    };

    /// Create a workspace → agent_definition → workspace_agent → session, and
    /// return `(session_id, instance_id)` — both valid FK targets.
    async fn fixture(pool: &SqlitePool) -> (String, String) {
        let ws = workspace::create(pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            pool,
            &AgentDefinitionInput {
                name: "FusionTestAgent".into(),
                role: None,
                agent_type: "chat".into(),
                cli_kind: None,
                color: None,
                provider_id: None,
                model: Some("m".into()),
                harness_mode: "own".into(),
                share_blackboard: None,
                auto_submit_injected: None,
                allowed_senders: None,
            },
        )
        .await
        .expect("create agent_def failed");
        let wa = workspace_agent::instantiate(pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed");
        let sid = session::get_by_instance(pool, &wa.id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists")
            .id;
        (sid, wa.id)
    }

    /// create_run → get_run round-trip: fields preserved, optionals NULL.
    #[tokio::test]
    async fn create_run_then_get() {
        let pool = connect_in_memory().await;
        let (sid, _wa) = fixture(&pool).await;

        let run = create_run(&pool, &sid, "What is best?")
            .await
            .expect("create_run failed");

        assert_eq!(run.session_id, sid);
        assert_eq!(run.prompt, "What is best?");
        assert!(run.judge_analysis.is_none());
        assert!(run.synthesized.is_none());
        assert!(!run.id.is_empty());
        assert!(!run.created_at.is_empty());

        let fetched = get_run(&pool, &run.id)
            .await
            .expect("get_run failed")
            .expect("run should exist");
        assert_eq!(fetched, run);

        let missing = get_run(&pool, "no-such-run").await.expect("get_run failed");
        assert!(missing.is_none(), "unknown run id must yield None");
    }

    /// create_panel_response (done + error) → list_responses, oldest-first.
    #[tokio::test]
    async fn panel_responses_done_and_error() {
        let pool = connect_in_memory().await;
        let (sid, wa) = fixture(&pool).await;
        let run = create_run(&pool, &sid, "Q")
            .await
            .expect("create_run failed");

        let done = create_panel_response(&pool, &run.id, &wa, Some("an answer"), "done")
            .await
            .expect("create done response failed");
        assert_eq!(done.status, "done");
        assert_eq!(done.answer.as_deref(), Some("an answer"));
        assert_eq!(done.instance_id.as_deref(), Some(wa.as_str()));

        let err = create_panel_response(&pool, &run.id, &wa, Some("boom: no key"), "error")
            .await
            .expect("create error response failed");
        assert_eq!(err.status, "error");
        assert!(err.answer.as_deref().unwrap().contains("boom"));

        let rows = list_responses(&pool, &run.id)
            .await
            .expect("list_responses failed");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, done.id, "oldest-first");
        assert_eq!(rows[1].id, err.id);
    }

    /// set_judge_analysis / set_synthesized are reflected by a subsequent get_run.
    #[tokio::test]
    async fn set_judge_and_synthesized_reflected() {
        let pool = connect_in_memory().await;
        let (sid, _wa) = fixture(&pool).await;
        let run = create_run(&pool, &sid, "Q")
            .await
            .expect("create_run failed");

        set_judge_analysis(&pool, &run.id, r#"{"consensus":"c"}"#)
            .await
            .expect("set_judge_analysis failed");
        set_synthesized(&pool, &run.id, "the final answer")
            .await
            .expect("set_synthesized failed");

        let fetched = get_run(&pool, &run.id)
            .await
            .expect("get_run failed")
            .expect("run exists");
        assert_eq!(
            fetched.judge_analysis.as_deref(),
            Some(r#"{"consensus":"c"}"#)
        );
        assert_eq!(fetched.synthesized.as_deref(), Some("the final answer"));
    }

    /// JSON serialization uses camelCase — locks the TS FusionRun /
    /// FusionPanelResponse contracts; None optionals are absent (not null).
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;
        let (sid, wa) = fixture(&pool).await;

        // Fresh run: judgeAnalysis / synthesized are None → absent.
        let run = create_run(&pool, &sid, "Q")
            .await
            .expect("create_run failed");
        let json = serde_json::to_value(&run).expect("serialize failed");
        assert!(json.get("sessionId").is_some(), "must have sessionId");
        assert!(json.get("createdAt").is_some(), "must have createdAt");
        assert!(json.get("session_id").is_none(), "no snake_case session_id");
        assert!(json.get("created_at").is_none(), "no snake_case created_at");
        assert!(
            json.get("judgeAnalysis").is_none(),
            "judgeAnalysis absent when None"
        );
        assert!(
            json.get("synthesized").is_none(),
            "synthesized absent when None"
        );

        // After setting the analysis, the camelCase key appears.
        set_judge_analysis(&pool, &run.id, r#"{"consensus":"c"}"#)
            .await
            .expect("set_judge_analysis failed");
        let fetched = get_run(&pool, &run.id)
            .await
            .expect("get_run failed")
            .expect("run exists");
        let json2 = serde_json::to_value(&fetched).expect("serialize failed");
        assert!(
            json2.get("judgeAnalysis").is_some(),
            "judgeAnalysis present once set"
        );

        // Panel response: fusionRunId / instanceId camelCase; snake absent.
        let resp = create_panel_response(&pool, &run.id, &wa, Some("a"), "done")
            .await
            .expect("create panel response failed");
        let rjson = serde_json::to_value(&resp).expect("serialize failed");
        assert!(rjson.get("fusionRunId").is_some(), "must have fusionRunId");
        assert!(rjson.get("instanceId").is_some(), "must have instanceId");
        assert!(rjson.get("fusion_run_id").is_none(), "no fusion_run_id");
        assert!(rjson.get("instance_id").is_none(), "no instance_id");
    }
}

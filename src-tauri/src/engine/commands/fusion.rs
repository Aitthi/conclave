//! Fusion orchestrator (M4.3) — panel fan-out → judge → synthesize.
//!
//! `fusion.run` fans a prompt out to a PANEL of agents (concurrently), asks a
//! JUDGE model to analyze the panel answers into structured JSON, then runs a
//! SYNTHESIZE step that produces one final answer. Everything is persisted into
//! `fusion_run` / `fusion_panel_response`, and `fusion:stage` events report
//! `panel` / `judge` / `synthesize` progress.
//!
//! # honesty seams (this repo forbids fabricated data)
//!
//! - Real provider calls need API keys + network, which tests/CI lack. The
//!   pipeline is therefore driven through a [`ModelCaller`] abstraction:
//!   production uses [`ModelCaller::Live`] (the real `Provider`); tests use
//!   `ModelCaller::Mock` with canned per-model results and NO network.
//! - A panel member that can't run (provider not configured / no key / HTTP
//!   error) is persisted with `status = "error"` and the honest error string in
//!   `answer` — never a fabricated answer. The run continues with the members
//!   that DID answer.
//! - The judge is asked for STRICT JSON; [`parse_judge_analysis`] parses
//!   defensively (strips ```json fences) and stores an honest
//!   `{ "raw", "parseError": true }` fallback on parse failure — it never
//!   invents structured fields. A judge CALL error stores
//!   `{ "error", "parseError": true }`.
//! - The M4.3 panel is DERIVED from the workspace's chat agents. Explicit
//!   `fusion_config` / `fusion_panel_member` selection (the config drawer) is
//!   M4.4.

use crate::engine::{bus, repo, AppError, AppState};
use repo::fusion::FusionRunRow;
use repo::{agent_definition, session, workspace_agent};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tauri::AppHandle;

/// Maximum number of panel members fanned out per run (M4.3 cap).
const MAX_PANEL: usize = 8;

// ── Request type ────────────────────────────────────────────────────────────

/// Payload for `fusion.run` — `{ orchestratorId, prompt }`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunReq {
    orchestrator_id: String,
    prompt: String,
}

/// Payload for `fusion.get` — `{ runId }`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetReq {
    run_id: String,
}

// ── Panel member ────────────────────────────────────────────────────────────

/// One resolved panel member: the instance to attribute the answer to, its
/// display `name`, and the `(provider_id, model)` used to call it.
struct PanelMember {
    instance_id: String,
    name: String,
    provider_id: Option<String>,
    model: String,
}

// ── Model-caller abstraction (testability — NO async-trait, NO network) ──────

/// How the pipeline obtains a completion. Production uses [`ModelCaller::Live`]
/// (the real `Provider`); tests use `Mock` with canned per-model results.
pub enum ModelCaller {
    /// Real provider call via `Provider::from_config` + `complete_chat`.
    Live,
    /// Test-only: map of model name → canned `Ok(answer)` / `Err(error)`.
    #[cfg(test)]
    Mock(std::collections::HashMap<String, Result<String, String>>),
}

impl ModelCaller {
    /// Run one completion. Returns `Ok(answer)` or `Err(honest error string)`.
    async fn call(
        &self,
        provider_id: Option<&str>,
        model: &str,
        prompt: &str,
    ) -> Result<String, String> {
        match self {
            ModelCaller::Live => {
                let p = crate::engine::runtime::provider::Provider::from_config(provider_id)
                    .map_err(|e| e.to_string())?;
                p.complete_chat(model, prompt)
                    .await
                    .map_err(|e| e.to_string())
            }
            #[cfg(test)]
            ModelCaller::Mock(map) => map
                .get(model)
                .cloned()
                .unwrap_or_else(|| Ok(format!("[mock answer for {model}]"))),
        }
    }
}

// ── Pure prompt builders / parser (unit-testable) ────────────────────────────

/// Build the JUDGE prompt: embeds the question + each panelist's name & answer
/// and instructs the model to return STRICT JSON with the agreed fields.
fn build_judge_prompt(question: &str, answers: &[(String, String)]) -> String {
    let mut s = String::new();
    s.push_str("You are an impartial judge analyzing a panel of expert answers to a question.\n\n");
    s.push_str("Question:\n");
    s.push_str(question);
    s.push_str("\n\nPanel answers:\n");
    if answers.is_empty() {
        s.push_str("(no panel members were available to answer)\n");
    } else {
        for (i, (name, answer)) in answers.iter().enumerate() {
            s.push_str(&format!("[Panelist {} — {name}]\n{answer}\n\n", i + 1));
        }
    }
    s.push_str(
        "Analyze the panel and return ONLY a JSON object (no prose, no markdown fences) with \
         exactly these fields:\n\
         {\n\
         \x20 \"consensus\": string,        // where the panelists agree\n\
         \x20 \"disagreements\": string[],  // points of conflict\n\
         \x20 \"insights\": string[],       // notable ideas worth keeping\n\
         \x20 \"blindSpots\": string[]      // gaps nobody addressed\n\
         }\n\
         Return ONLY a JSON object, no prose.",
    );
    s
}

/// Build the SYNTHESIZE prompt: produce a single best final answer from the
/// panel answers and the judge's analysis JSON.
fn build_synthesis_prompt(
    question: &str,
    answers: &[(String, String)],
    judge_json: &str,
) -> String {
    let mut s = String::new();
    s.push_str(
        "You are synthesizing a single best final answer from a panel of expert answers and a \
         judge's structured analysis.\n\n",
    );
    s.push_str("Question:\n");
    s.push_str(question);
    s.push_str("\n\nPanel answers:\n");
    if answers.is_empty() {
        s.push_str("(no panel members were available)\n");
    } else {
        for (i, (name, answer)) in answers.iter().enumerate() {
            s.push_str(&format!("[Panelist {} — {name}]\n{answer}\n\n", i + 1));
        }
    }
    s.push_str("Judge analysis (JSON):\n");
    s.push_str(judge_json);
    s.push_str(
        "\n\nWrite one clear, complete final answer to the question, integrating the strongest \
         points and resolving disagreements where possible. Return prose only (no JSON).",
    );
    s
}

/// Strip a leading ```/```json fence and its closing fence, returning the inner
/// slice. Returns the trimmed input unchanged when there is no opening fence.
fn strip_code_fences(text: &str) -> &str {
    let mut s = text.trim();
    if let Some(rest) = s.strip_prefix("```") {
        s = rest;
        // Optional language hint immediately after the opening fence (any case,
        // e.g. ```json / ```JSON) — a valid-JSON body shouldn't go unparsed just
        // because the model upper-cased the hint.
        if let Some(after) = s.strip_prefix("json").or_else(|| s.strip_prefix("JSON")) {
            s = after;
        }
        // Drop the closing fence if present.
        if let Some(idx) = s.rfind("```") {
            s = &s[..idx];
        }
    }
    s.trim()
}

/// Parse the judge's reply into a JSON value. Strips ```json fences and parses;
/// on parse failure returns an HONEST fallback `{ "raw": <text>, "parseError":
/// true }` — it never invents structured fields.
pub fn parse_judge_analysis(text: &str) -> Value {
    let cleaned = strip_code_fences(text);
    match serde_json::from_str::<Value>(cleaned) {
        Ok(v) => v,
        Err(_) => json!({ "raw": text, "parseError": true }),
    }
}

// ── Emit helper ──────────────────────────────────────────────────────────────

/// Emit a `fusion:stage` event when an `AppHandle` is available (skipped in
/// tests, where `app` is `None`).
fn emit_stage(app: Option<&AppHandle>, run_id: &str, stage: &str, data: Option<Value>) {
    if let Some(app) = app {
        let _ = bus::fusion_stage(
            app,
            bus::FusionStage {
                run_id: run_id.to_owned(),
                stage: stage.to_owned(),
                data,
            },
        );
    }
}

// ── Panel derivation (testable without network) ──────────────────────────────

/// Derive the panel from the workspace's chat agents (M4.3): every instance in
/// `workspace_id` whose definition is `type == "chat"` AND has a `model`, except
/// the orchestrator itself. Capped at [`MAX_PANEL`], keeping the first ones by
/// the workspace's existing instance order.
///
/// Explicit `fusion_config` / `fusion_panel_member` selection is M4.4; M4.3
/// derives the panel here.
async fn derive_panel(
    db: &SqlitePool,
    orchestrator_id: &str,
    workspace_id: &str,
) -> Result<Vec<PanelMember>, AppError> {
    let instances = workspace_agent::list_by_workspace(db, workspace_id).await?;
    let mut panel = Vec::new();
    for wa in instances {
        if wa.id == orchestrator_id {
            continue; // the orchestrator is the judge, never a panelist
        }
        let Some(def) = agent_definition::get(db, &wa.agent_def_id).await? else {
            continue;
        };
        if def.r#type != "chat" {
            continue;
        }
        let Some(model) = def.model else {
            continue; // chat agent with no model can't be called honestly
        };
        panel.push(PanelMember {
            instance_id: wa.id,
            name: def.name,
            provider_id: def.provider_id,
            model,
        });
        if panel.len() >= MAX_PANEL {
            break;
        }
    }
    Ok(panel)
}

// ── Pipeline (steps 5–9) ──────────────────────────────────────────────────────

/// Run the persist + fan-out + judge + synthesize pipeline. Factored out of
/// [`run`] so tests can drive it with `ModelCaller::Mock` and `app = None`.
///
/// `judge_ref` is the orchestrator's `(provider_id, model)` — used for BOTH the
/// judge and the synthesize calls.
async fn run_pipeline(
    db: &SqlitePool,
    app: Option<&AppHandle>,
    caller: &ModelCaller,
    session_id: &str,
    prompt: &str,
    judge_ref: (Option<String>, String),
    panel: Vec<PanelMember>,
) -> Result<FusionRunRow, AppError> {
    // 5. Create the run + emit the "panel" stage.
    let run = repo::fusion::create_run(db, session_id, prompt).await?;
    let run_id = run.id.clone();
    emit_stage(
        app,
        &run_id,
        "panel",
        Some(json!({
            "count": panel.len(),
            "members": panel.iter().map(|m| m.name.clone()).collect::<Vec<_>>(),
        })),
    );

    // 6. Panel fan-out (concurrent). The futures borrow `caller` / `prompt`
    //    immutably (`&self`) — safe to run in parallel.
    let calls = panel.iter().map(|m| async move {
        let res = caller
            .call(m.provider_id.as_deref(), &m.model, prompt)
            .await;
        (m, res)
    });
    let results = futures_util::future::join_all(calls).await;

    // Persist each response honestly; collect the successful (name, answer) pairs.
    // A single panel-row write failure is best-effort (logged, not propagated):
    // one flaky INSERT must not abort the whole run and strand the `fusion_run`
    // as an orphan with NULL judge/synthesis. The successful answer is still fed
    // to the judge even if its row failed to persist.
    let mut answers: Vec<(String, String)> = Vec::new();
    for (member, res) in results {
        let (answer_opt, status): (Option<&str>, &str) = match &res {
            Ok(answer) => (Some(answer.as_str()), "done"),
            // HONEST error row — the real error string, never a fake answer.
            Err(err) => (Some(err.as_str()), "error"),
        };
        if let Err(e) = repo::fusion::create_panel_response(
            db,
            &run_id,
            &member.instance_id,
            answer_opt,
            status,
        )
        .await
        {
            eprintln!(
                "fusion: failed to persist panel response for instance {}: {e}",
                member.instance_id
            );
        }
        if let Ok(answer) = res {
            answers.push((member.name.clone(), answer));
        }
    }

    // 7. Judge: analyze the successful answers into structured JSON.
    let judge_prompt = build_judge_prompt(prompt, &answers);
    let analysis = match caller
        .call(judge_ref.0.as_deref(), &judge_ref.1, &judge_prompt)
        .await
    {
        Ok(text) => parse_judge_analysis(&text),
        // The judge call itself failed — store an HONEST error, keep going.
        Err(err) => json!({ "error": err, "parseError": true }),
    };
    // Serialise once, reuse for the DB write, the event, and the synthesis prompt.
    let analysis_str = analysis.to_string();
    repo::fusion::set_judge_analysis(db, &run_id, &analysis_str).await?;
    emit_stage(app, &run_id, "judge", Some(analysis));

    // 8. Synthesize: one final answer from the panel + judge analysis.
    let synth_prompt = build_synthesis_prompt(prompt, &answers, &analysis_str);
    let synthesized = match caller
        .call(judge_ref.0.as_deref(), &judge_ref.1, &synth_prompt)
        .await
    {
        Ok(text) => text,
        Err(err) => format!("[synthesis failed: {err}]"),
    };
    repo::fusion::set_synthesized(db, &run_id, &synthesized).await?;
    // Don't dump the whole answer in the event — it's persisted.
    emit_stage(app, &run_id, "synthesize", Some(json!({ "ok": true })));

    // 9. Re-fetch the run and return it (matches the TS `FusionRun`).
    repo::fusion::get_run(db, &run_id)
        .await?
        .ok_or_else(|| AppError::Internal("fusion run vanished after creation".into()))
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// `fusion.run` — orchestrate a fusion run and return the resulting `FusionRun`.
pub async fn run(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: RunReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    if req.prompt.trim().is_empty() {
        return Err(AppError::Invalid("prompt must not be empty".into()));
    }
    let db = &state.db;

    // 2. Orchestrator instance + its definition (the judge/synthesis model).
    let orchestrator = workspace_agent::get(db, &req.orchestrator_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "orchestrator instance id={} not found",
                req.orchestrator_id
            ))
        })?;
    let orch_def = agent_definition::get(db, &orchestrator.agent_def_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "orchestrator agent definition id={} not found",
                orchestrator.agent_def_id
            ))
        })?;
    // `orch_def` is not used after this — move its fields rather than clone.
    let judge_model = orch_def
        .model
        .ok_or_else(|| AppError::Invalid("orchestrator has no model configured".into()))?;
    let judge_ref = (orch_def.provider_id, judge_model);

    // 3. Resolve the orchestrator's session → fusion_run.session_id.
    let session = session::get_by_instance(db, &req.orchestrator_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "session for instance id={} not found",
                req.orchestrator_id
            ))
        })?;

    // 4. Derive the panel from the workspace's chat agents (empty is OK — the
    //    pipeline records reality and does not error).
    let panel = derive_panel(db, &req.orchestrator_id, &orchestrator.workspace_id).await?;

    let row = run_pipeline(
        db,
        state.app(),
        &ModelCaller::Live,
        &session.id,
        &req.prompt,
        judge_ref,
        panel,
    )
    .await?;

    serde_json::to_value(&row).map_err(|e| AppError::Internal(e.to_string()))
}

/// `fusion.get` — load a persisted run and its panel responses for the UI.
///
/// Returns `{ "run": <FusionRunRow>, "responses": [<FusionPanelResponseRow>] }`.
/// The run renders REAL data: panel answers (a member that couldn't run shows
/// `status = "error"` with the honest error string), the run's `judgeAnalysis`
/// JSON string, and the run's real `synthesized` answer.
pub async fn get(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: GetReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let db = &state.db;

    let run = repo::fusion::get_run(db, &req.run_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("fusion run id={} not found", req.run_id)))?;
    let responses = repo::fusion::list_responses(db, &req.run_id).await?;

    Ok(json!({ "run": run, "responses": responses }))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        session, workspace, workspace_agent,
    };
    use std::collections::HashMap;

    // ── Pure-helper tests ────────────────────────────────────────────────────

    #[test]
    fn parse_judge_analysis_clean_json() {
        let v = parse_judge_analysis(r#"{"consensus":"agree","disagreements":[]}"#);
        assert_eq!(v.get("consensus").and_then(Value::as_str), Some("agree"));
        assert!(
            v.get("parseError").is_none(),
            "clean JSON is not a fallback"
        );
    }

    #[test]
    fn parse_judge_analysis_fenced_json() {
        let fenced = "```json\n{\"consensus\":\"c\",\"insights\":[\"x\"]}\n```";
        let v = parse_judge_analysis(fenced);
        assert_eq!(v.get("consensus").and_then(Value::as_str), Some("c"));
        // Also tolerate a bare ``` fence with no language hint.
        let bare = "```\n{\"consensus\":\"c\"}\n```";
        assert_eq!(
            parse_judge_analysis(bare)
                .get("consensus")
                .and_then(Value::as_str),
            Some("c")
        );
    }

    #[test]
    fn parse_judge_analysis_non_json_fallback() {
        let v = parse_judge_analysis("I think the panel mostly agreed.");
        assert_eq!(v.get("parseError").and_then(Value::as_bool), Some(true));
        assert_eq!(
            v.get("raw").and_then(Value::as_str),
            Some("I think the panel mostly agreed.")
        );
        assert!(
            v.get("consensus").is_none(),
            "fallback must NOT invent structured fields"
        );
    }

    #[test]
    fn build_prompts_embed_question_and_answers() {
        let answers = vec![("Ada".to_string(), "use a queue".to_string())];
        let jp = build_judge_prompt("How to scale?", &answers);
        assert!(
            jp.contains("How to scale?"),
            "judge prompt has the question"
        );
        assert!(jp.contains("use a queue"), "judge prompt has the answer");
        assert!(jp.contains("Ada"), "judge prompt names the panelist");
        assert!(jp.contains("JSON"), "judge prompt demands JSON");

        let sp = build_synthesis_prompt("How to scale?", &answers, r#"{"consensus":"c"}"#);
        assert!(
            sp.contains("How to scale?"),
            "synth prompt has the question"
        );
        assert!(sp.contains("use a queue"), "synth prompt has the answer");
        assert!(
            sp.contains(r#"{"consensus":"c"}"#),
            "synth prompt has judge JSON"
        );
    }

    // ── Integration fixtures ─────────────────────────────────────────────────

    fn chat_input(name: &str, provider: Option<&str>, model: Option<&str>) -> AgentDefinitionInput {
        AgentDefinitionInput {
            name: name.to_owned(),
            role: None,
            agent_type: "chat".into(),
            cli_kind: None,
            color: None,
            provider_id: provider.map(|s| s.to_owned()),
            model: model.map(|s| s.to_owned()),
            harness_mode: "own".into(),
            share_blackboard: None,
            auto_submit_injected: None,
            allowed_senders: None,
            ..Default::default()
        }
    }

    /// Create an instance for `(workspace, definition-input)` and return its id.
    async fn instance_in(state: &AppState, ws_id: &str, input: &AgentDefinitionInput) -> String {
        let def = agent_definition::create(&state.db, input)
            .await
            .expect("create agent_def failed");
        workspace_agent::instantiate(&state.db, ws_id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    /// derive_panel keeps only chat+model agents, excludes the orchestrator and
    /// non-chat agents, and the orchestrator never appears as a panelist.
    #[tokio::test]
    async fn derive_panel_excludes_orchestrator_and_non_chat() {
        let state = AppState::for_tests().await;
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");

        let orch = instance_in(
            &state,
            &ws.id,
            &chat_input("Orch", None, Some("judge-model")),
        )
        .await;
        let p1 = instance_in(&state, &ws.id, &chat_input("Panel1", None, Some("m1"))).await;
        let _p2 = instance_in(&state, &ws.id, &chat_input("Panel2", None, Some("m2"))).await;
        // A chat agent with NO model is excluded.
        let _no_model = instance_in(&state, &ws.id, &chat_input("NoModel", None, None)).await;
        // A non-chat (cli) agent is excluded even if it has a model.
        let mut cli = chat_input("CliAgent", None, Some("cli-model"));
        cli.agent_type = "cli".into();
        let _cli = instance_in(&state, &ws.id, &cli).await;

        let panel = derive_panel(&state.db, &orch, &ws.id)
            .await
            .expect("derive_panel failed");

        assert_eq!(panel.len(), 2, "only the 2 chat+model panelists");
        assert!(
            panel.iter().all(|m| m.instance_id != orch),
            "orchestrator must NOT be a panel member"
        );
        assert!(panel.iter().any(|m| m.instance_id == p1));
        let names: Vec<&str> = panel.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"Panel1") && names.contains(&"Panel2"));
    }

    /// Full pipeline with a Mock caller: one panelist answers, one fails; the
    /// run persists judge_analysis + synthesized and both panel rows.
    #[tokio::test]
    async fn pipeline_with_mock_panel() {
        let state = AppState::for_tests().await;
        let db = &state.db;
        let ws = workspace::create(db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");

        let orch = instance_in(
            &state,
            &ws.id,
            &chat_input("Orch", None, Some("judge-model")),
        )
        .await;
        let session = session::get_by_instance(db, &orch)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        let p1 = instance_in(&state, &ws.id, &chat_input("Panel1", None, Some("m1"))).await;
        let p2 = instance_in(&state, &ws.id, &chat_input("Panel2", None, Some("m2"))).await;

        let panel = vec![
            PanelMember {
                instance_id: p1.clone(),
                name: "Panel1".into(),
                provider_id: None,
                model: "m1".into(),
            },
            PanelMember {
                instance_id: p2.clone(),
                name: "Panel2".into(),
                provider_id: None,
                model: "m2".into(),
            },
        ];

        let mut map: HashMap<String, Result<String, String>> = HashMap::new();
        map.insert("m1".into(), Ok("answer one".into()));
        map.insert("m2".into(), Err("boom".into()));
        map.insert(
            "judge-model".into(),
            Ok(r#"{"consensus":"c","disagreements":[],"insights":[],"blindSpots":[]}"#.into()),
        );
        let caller = ModelCaller::Mock(map);

        let row = run_pipeline(
            db,
            None,
            &caller,
            &session.id,
            "What is best?",
            (None, "judge-model".into()),
            panel,
        )
        .await
        .expect("run_pipeline failed");

        // judge_analysis non-null, parses to an object with "consensus".
        let analysis: Value =
            serde_json::from_str(row.judge_analysis.as_deref().expect("judge_analysis set"))
                .expect("judge_analysis is JSON");
        assert_eq!(analysis.get("consensus").and_then(Value::as_str), Some("c"));
        // synthesized must hold the synth call's RESULT (the canned answer for
        // "judge-model"), proving the synthesis step actually ran and persisted
        // its output — not the "[synthesis failed: …]" honest fallback.
        assert_eq!(
            row.synthesized.as_deref(),
            Some(r#"{"consensus":"c","disagreements":[],"insights":[],"blindSpots":[]}"#),
            "synthesized must be the synth call result, not a failure marker"
        );

        // Exactly 2 panel rows: one done ("answer one"), one error (contains "boom").
        let responses = repo::fusion::list_responses(db, &row.id)
            .await
            .expect("list_responses failed");
        assert_eq!(responses.len(), 2);
        let done = responses
            .iter()
            .find(|r| r.status == "done")
            .expect("a done response");
        assert_eq!(done.answer.as_deref(), Some("answer one"));
        let err = responses
            .iter()
            .find(|r| r.status == "error")
            .expect("an error response");
        assert!(err.answer.as_deref().unwrap().contains("boom"));

        // The orchestrator was NOT counted as a panel member.
        assert!(
            responses
                .iter()
                .all(|r| r.instance_id.as_deref() != Some(orch.as_str())),
            "orchestrator must not appear among panel responses"
        );
    }

    /// Empty panel (orchestrator only): the run is still created with non-null
    /// judge/synthesized and zero panel responses — reality recorded, no error.
    #[tokio::test]
    async fn pipeline_with_empty_panel() {
        let state = AppState::for_tests().await;
        let db = &state.db;
        let ws = workspace::create(db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let orch = instance_in(
            &state,
            &ws.id,
            &chat_input("Orch", None, Some("judge-model")),
        )
        .await;
        let session = session::get_by_instance(db, &orch)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        let mut map: HashMap<String, Result<String, String>> = HashMap::new();
        map.insert(
            "judge-model".into(),
            Ok(r#"{"consensus":"none","disagreements":[],"insights":[],"blindSpots":[]}"#.into()),
        );
        let caller = ModelCaller::Mock(map);

        let row = run_pipeline(
            db,
            None,
            &caller,
            &session.id,
            "Anything?",
            (None, "judge-model".into()),
            vec![],
        )
        .await
        .expect("run_pipeline failed");

        assert!(row.judge_analysis.is_some(), "judge_analysis set");
        assert!(row.synthesized.is_some(), "synthesized set");
        let responses = repo::fusion::list_responses(db, &row.id)
            .await
            .expect("list_responses failed");
        assert!(responses.is_empty(), "no panel members → no responses");
    }

    /// run() with an unknown orchestratorId → NotFound (no fabricated run).
    #[tokio::test]
    async fn run_unknown_orchestrator_not_found() {
        let state = AppState::for_tests().await;
        let err = run(&state, json!({ "orchestratorId": "nope", "prompt": "hi" }))
            .await
            .expect_err("run must fail for an unknown orchestrator");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// run() when the orchestrator definition has no model → Invalid.
    #[tokio::test]
    async fn run_orchestrator_without_model_invalid() {
        let state = AppState::for_tests().await;
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        // Orchestrator chat agent with model = None.
        let orch = instance_in(&state, &ws.id, &chat_input("Orch", None, None)).await;

        let err = run(&state, json!({ "orchestratorId": orch, "prompt": "hi" }))
            .await
            .expect_err("run must fail when orchestrator has no model");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    /// run() with an empty prompt → Invalid (before any lookups).
    #[tokio::test]
    async fn run_empty_prompt_invalid() {
        let state = AppState::for_tests().await;
        let err = run(&state, json!({ "orchestratorId": "x", "prompt": "  " }))
            .await
            .expect_err("empty prompt must be rejected");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    /// get() returns the persisted run + its panel responses (the UI read path).
    #[tokio::test]
    async fn get_returns_run_and_responses() {
        let state = AppState::for_tests().await;
        let db = &state.db;
        let ws = workspace::create(db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let orch = instance_in(
            &state,
            &ws.id,
            &chat_input("Orch", None, Some("judge-model")),
        )
        .await;
        let session = session::get_by_instance(db, &orch)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        // Create a run + 2 panel responses via the repo.
        let run = repo::fusion::create_run(db, &session.id, "Q")
            .await
            .expect("create_run failed");
        repo::fusion::create_panel_response(db, &run.id, &orch, Some("an answer"), "done")
            .await
            .expect("create done response failed");
        repo::fusion::create_panel_response(db, &run.id, &orch, Some("boom"), "error")
            .await
            .expect("create error response failed");

        let out = get(&state, json!({ "runId": run.id }))
            .await
            .expect("get failed");

        assert_eq!(
            out.get("run")
                .and_then(|r| r.get("id"))
                .and_then(Value::as_str),
            Some(run.id.as_str()),
            "returned run.id must match"
        );
        let responses = out
            .get("responses")
            .and_then(Value::as_array)
            .expect("responses is an array");
        assert_eq!(responses.len(), 2, "both panel responses returned");
    }

    /// get() with an unknown runId → NotFound (no fabricated run).
    #[tokio::test]
    async fn get_unknown_run_not_found() {
        let state = AppState::for_tests().await;
        let err = get(&state, json!({ "runId": "nope" }))
            .await
            .expect_err("get must fail for an unknown run");
        assert!(matches!(err, AppError::NotFound(_)));
    }
}

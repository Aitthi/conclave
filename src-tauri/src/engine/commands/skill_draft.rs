//! Skill-draft agent-assist sessions — spawn a real CLI agent (reusing the
//! existing workspace/session/instance machinery via a HIDDEN, single-purpose
//! `Workspace`) against a scratch `SKILL.md` so it can write a custom skill's
//! name/description/content directly. See
//! docs/specs/2026-07-02-skill-editor-agent-assist-design.md.

use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartDraftReq {
    name: String,
    description: Option<String>,
    content: String,
    agent_def_id: String,
}

/// Start an agent-assist draft session: materialize the skill's current
/// fields as a scratch `SKILL.md`, create a hidden `Workspace` +
/// `workspace_agent` + `session` pointed at it, and spawn the chosen
/// `AgentDefinition` exactly as a real workspace would (reuses
/// `instance::spawn` unmodified). Maps to `skill.startDraftSession`.
///
/// Rejects a non-`cli` or unconfigured/unsupported-`cli_kind` agent
/// definition BEFORE creating any resources — `instance::spawn`'s `chat` and
/// `orchestrator` branches don't give the agent real file tool access, and
/// only `claude-code`/`codex`/`antigravity` are launchable `cli_kind`s today (see
/// `instance::spawn`'s own dispatch) — failing fast here avoids leaving an
/// orphaned hidden workspace + scratch dir behind.
fn validate_skill_assist_agent(agent_type: &str, cli_kind: Option<&str>) -> Result<(), AppError> {
    if agent_type == "cli"
        && matches!(
            cli_kind,
            Some("claude-code") | Some("codex") | Some("antigravity")
        )
    {
        return Ok(());
    }
    Err(AppError::Invalid(
        "skill-assist agent must be a configured CLI agent (Claude Code, Codex, or Antigravity)"
            .into(),
    ))
}

pub async fn start(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: StartDraftReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let def = repo::agent_definition::get(&state.db, &req.agent_def_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "agent_definition id={} not found",
                req.agent_def_id
            ))
        })?;
    validate_skill_assist_agent(&def.r#type, def.cli_kind.as_deref())?;

    let dir = repo::skill::new_draft_dir()
        .map_err(|e| AppError::Internal(format!("create skill draft scratch dir: {e}")))?;
    repo::skill::write_draft(&dir, &req.name, req.description.as_deref(), &req.content)
        .map_err(|e| AppError::Internal(format!("write skill draft: {e}")))?;

    let ws =
        match repo::workspace::create_hidden(&state.db, &req.name, &dir.to_string_lossy()).await {
            Ok(ws) => ws,
            Err(e) => {
                // Best-effort rollback: a doomed create_hidden must not leave the
                // just-written scratch dir behind for the user to never see again.
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e.into());
            }
        };
    let wa = match repo::workspace_agent::instantiate(&state.db, &ws.id, &req.agent_def_id).await {
        Ok(wa) => wa,
        Err(e) => {
            // Best-effort rollback: a doomed instantiate must not leave the
            // just-created hidden workspace + scratch dir behind for the user
            // to never see again.
            let _ = super::workspace::delete(state, json!({ "workspaceId": ws.id })).await;
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e.into());
        }
    };

    match super::instance::spawn(state, json!({ "workspaceAgentId": wa.id })).await {
        Ok(session) => {
            let session_id = session
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Internal("spawn returned no session id".into()))?
                .to_owned();
            Ok(json!({ "workspaceAgentId": wa.id, "sessionId": session_id }))
        }
        Err(e) => {
            // Best-effort rollback: a doomed spawn must not leave a hidden
            // workspace + scratch dir behind for the user to never see again.
            let _ = super::workspace::delete(state, json!({ "workspaceId": ws.id })).await;
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftSessionReq {
    workspace_agent_id: String,
}

/// Re-read + parse the draft session's scratch `SKILL.md`, returning the
/// latest `{name, description, content}`. Maps to `skill.syncDraft`.
///
/// If the file is currently missing or unparsable (e.g. the agent is
/// mid-write), returns `AppError::Invalid` — the FRONTEND simply keeps
/// showing the last successfully synced values on a failed sync (it only
/// applies a sync's result when the call succeeds), so no "last good" state
/// needs to be tracked here.
pub async fn sync(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DraftSessionReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let wa = repo::workspace_agent::get(&state.db, &req.workspace_agent_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "workspace_agent id={} not found",
                req.workspace_agent_id
            ))
        })?;
    let ws = repo::workspace::get(&state.db, &wa.workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace id={} not found", wa.workspace_id)))?;

    let (name, description, content) =
        repo::skill::read_draft(std::path::Path::new(&ws.folder_path)).ok_or_else(|| {
            AppError::Invalid("skill draft file is missing or unparsable right now".into())
        })?;

    // `description` is OMITTED (not `null`) when absent, matching this
    // codebase's `skip_serializing_if = "Option::is_none"` convention
    // elsewhere (e.g. `SkillRow.description`) and the frontend's `description?:
    // string` (not `string | null`) contract exactly.
    let mut res = json!({ "name": name, "content": content });
    if let Some(d) = description {
        res["description"] = json!(d);
    }
    Ok(res)
}

/// Stop a draft session's live agent, tear down its hidden workspace (and
/// everything that hangs off it, via `workspace::delete`), and remove the
/// scratch directory from disk. Maps to `skill.stopDraftSession`.
///
/// Idempotent: an unknown `workspaceAgentId` (already cleaned up) is a no-op,
/// matching `instance::stop`'s idempotent-on-not-live precedent — the
/// frontend calls this defensively on unmount, and an uncaught rejection
/// there shouldn't surface as an error to the user.
pub async fn stop(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DraftSessionReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let Some(wa) = repo::workspace_agent::get(&state.db, &req.workspace_agent_id).await? else {
        return Ok(Value::Null);
    };
    let folder_path = repo::workspace::get(&state.db, &wa.workspace_id)
        .await?
        .map(|ws| ws.folder_path);

    super::instance::stop(state, json!({ "workspaceAgentId": req.workspace_agent_id })).await?;
    super::workspace::delete(state, json!({ "workspaceId": wa.workspace_id })).await?;

    if let Some(path) = folder_path {
        let _ = std::fs::remove_dir_all(path);
    }

    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture_cli_agent_def(state: &AppState, cli_kind: Option<&str>) -> String {
        repo::agent_definition::create(
            &state.db,
            &repo::agent_definition::AgentDefinitionInput {
                name: "Assistant".into(),
                agent_type: "cli".into(),
                cli_kind: cli_kind.map(str::to_owned),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed")
        .id
    }

    /// Builds a hidden workspace + workspace_agent + session directly,
    /// bypassing `start`'s real CLI spawn (see this file's
    /// `start_rejects_*` tests and this plan's Global Constraints — spawning
    /// a real `claude`/`codex` process in a unit test repeats the exact
    /// "binary-free" boundary `commands::instance`'s own tests already
    /// respect). Returns `(workspace_agent_id, scratch_dir)`.
    async fn fixture_draft_session(
        state: &AppState,
        name: &str,
        description: Option<&str>,
        content: &str,
    ) -> (String, std::path::PathBuf) {
        let dir = repo::skill::new_draft_dir().expect("new_draft_dir failed");
        repo::skill::write_draft(&dir, name, description, content).expect("write_draft failed");

        let ws = repo::workspace::create_hidden(&state.db, name, &dir.to_string_lossy())
            .await
            .expect("create_hidden failed");
        let def_id = fixture_cli_agent_def(state, Some("claude-code")).await;
        let wa = repo::workspace_agent::instantiate(&state.db, &ws.id, &def_id)
            .await
            .expect("instantiate failed");
        (wa.id, dir)
    }

    #[tokio::test]
    async fn start_rejects_non_cli_agent_def() {
        let state = AppState::for_tests().await;
        let def = repo::agent_definition::create(
            &state.db,
            &repo::agent_definition::AgentDefinitionInput {
                name: "Orch".into(),
                agent_type: "orchestrator".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");

        let err = start(
            &state,
            json!({ "name": "X", "content": "c", "agentDefId": def.id }),
        )
        .await
        .expect_err("must reject a non-cli agent def");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn start_rejects_cli_agent_with_no_cli_kind() {
        let state = AppState::for_tests().await;
        let def_id = fixture_cli_agent_def(&state, None).await;

        let err = start(
            &state,
            json!({ "name": "X", "content": "c", "agentDefId": def_id }),
        )
        .await
        .expect_err("must reject a cli agent def with no cli_kind configured");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn start_rejects_unknown_agent_def() {
        let state = AppState::for_tests().await;
        let err = start(
            &state,
            json!({ "name": "X", "content": "c", "agentDefId": "no-such-def" }),
        )
        .await
        .expect_err("must reject an unknown agent_def id");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn skill_assist_guard_accepts_antigravity_and_rejects_unlaunchable_kinds() {
        validate_skill_assist_agent("cli", Some("antigravity"))
            .expect("Antigravity must pass before resources are created");

        for (agent_type, cli_kind) in [
            ("chat", Some("antigravity")),
            ("orchestrator", Some("antigravity")),
            ("cli", None),
            ("cli", Some("custom")),
            ("cli", Some("unknown")),
        ] {
            let error = validate_skill_assist_agent(agent_type, cli_kind)
                .expect_err("unlaunchable skill-assist agent must be rejected");
            assert!(matches!(error, AppError::Invalid(_)));
        }
    }

    #[tokio::test]
    async fn sync_reads_back_the_current_scratch_file() {
        let state = AppState::for_tests().await;
        let (wa_id, dir) = fixture_draft_session(&state, "Reviewer", Some("desc"), "Body").await;

        let result = sync(&state, json!({ "workspaceAgentId": wa_id }))
            .await
            .expect("sync failed");
        assert_eq!(result["name"], "Reviewer");
        assert_eq!(result["description"], "desc");
        assert_eq!(result["content"], "Body");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sync_reflects_a_file_edited_after_start() {
        let state = AppState::for_tests().await;
        let (wa_id, dir) = fixture_draft_session(&state, "Reviewer", None, "Original").await;

        // Simulate the agent having edited the scratch file.
        repo::skill::write_draft(
            &dir,
            "Reviewer v2",
            Some("now with a description"),
            "Updated body",
        )
        .expect("simulated agent edit failed");

        let result = sync(&state, json!({ "workspaceAgentId": wa_id }))
            .await
            .expect("sync failed");
        assert_eq!(result["name"], "Reviewer v2");
        assert_eq!(result["content"], "Updated body");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sync_unknown_workspace_agent_not_found() {
        let state = AppState::for_tests().await;
        let err = sync(&state, json!({ "workspaceAgentId": "nope" }))
            .await
            .expect_err("should fail for unknown id");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn stop_deletes_hidden_workspace_and_scratch_dir() {
        let state = AppState::for_tests().await;
        let (wa_id, dir) = fixture_draft_session(&state, "Doomed", None, "c").await;
        assert!(dir.is_dir());

        stop(&state, json!({ "workspaceAgentId": wa_id }))
            .await
            .expect("stop failed");

        assert!(
            !repo::workspace_agent::exists(&state.db, &wa_id)
                .await
                .expect("exists check failed"),
            "workspace_agent must be gone"
        );
        assert!(!dir.exists(), "scratch dir must be removed");
    }

    #[tokio::test]
    async fn stop_unknown_workspace_agent_is_idempotent_noop() {
        let state = AppState::for_tests().await;
        let result = stop(&state, json!({ "workspaceAgentId": "nope" })).await;
        assert_eq!(result.expect("must not error"), Value::Null);
    }

    #[tokio::test]
    async fn hidden_draft_workspace_never_appears_in_workspace_list() {
        let state = AppState::for_tests().await;
        let (_wa_id, dir) = fixture_draft_session(&state, "Invisible", None, "c").await;

        let listed = repo::workspace::list(&state.db).await.expect("list failed");
        assert!(
            listed.is_empty(),
            "a hidden draft-session workspace must never appear in workspace.list"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}

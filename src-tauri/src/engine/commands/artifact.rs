//! Artifact commands (plan design-artifact-store, Lane A) — persist and read
//! workspace-scoped agent outputs.
//!
//! `artifact.add` writes one artifact and emits `artifact:changed` so an open
//! Artifacts view refetches; `artifact.list`/`artifact.get` are read-only.
//! `agentId` is free text (a creator agent instance OR definition id — the CLI
//! resolves it), so unlike `task`/`blackboard` actors it is NOT scope-enforced;
//! `workspaceId` still must exist.

use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::Value;

/// The artifact `kind` allowlist (plan §4 / Decision 3). Kept here — the
/// command layer is the choke point every caller (ipc + `conclave artifact
/// add`) funnels through, so an unknown kind is rejected once, centrally.
pub const ARTIFACT_KINDS: [&str; 7] = ["markdown", "code", "html", "svg", "mermaid", "react", "text"];

/// Validate that `workspace_id` exists, else [`AppError::NotFound`] (mirrors
/// `commands::task::require_workspace`).
async fn require_workspace(state: &AppState, workspace_id: &str) -> Result<(), AppError> {
    if !repo::workspace::exists(&state.db, workspace_id).await? {
        return Err(AppError::NotFound(format!("workspace id={workspace_id} not found")));
    }
    Ok(())
}

/// Emit `artifact:changed` after a successful add. Non-fatal (mirrors every
/// other `bus::*` emit — a missed UI refresh is not a request failure).
fn emit_changed(state: &AppState, workspace_id: &str) {
    state.emit(
        crate::engine::bus::ARTIFACT_CHANGED,
        crate::engine::bus::ArtifactChanged {
            workspace_id: workspace_id.to_owned(),
        },
    );
}

// ── artifact.add ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddReq {
    workspace_id: String,
    agent_id: Option<String>,
    title: String,
    kind: String,
    filename: Option<String>,
    content: String,
}

/// Persist a workspace-scoped artifact. Rejects an unknown `kind` with the
/// allowed list, and a missing `workspaceId`.
pub async fn add(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: AddReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    if !ARTIFACT_KINDS.contains(&req.kind.as_str()) {
        return Err(AppError::Invalid(format!(
            "unknown kind '{}' (allowed: {})",
            req.kind,
            ARTIFACT_KINDS.join(", ")
        )));
    }

    let row = repo::artifact::insert_artifact(
        &state.db,
        &req.workspace_id,
        req.agent_id.as_deref(),
        &req.title,
        &req.kind,
        req.filename.as_deref(),
        &req.content,
    )
    .await?;

    emit_changed(state, &req.workspace_id);
    serde_json::to_value(&row).map_err(|e| AppError::Internal(e.to_string()))
}

// ── artifact.list ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListReq {
    workspace_id: String,
}

/// List a workspace's artifacts, newest first.
pub async fn list(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ListReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    let rows = repo::artifact::list_artifacts(&state.db, &req.workspace_id).await?;
    serde_json::to_value(&rows).map_err(|e| AppError::Internal(e.to_string()))
}

// ── artifact.get ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetReq {
    id: String,
}

/// Fetch one artifact by id, or [`AppError::NotFound`].
pub async fn get(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: GetReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let row = repo::artifact::get_artifact(&state.db, &req.id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("artifact id={} not found", req.id)))?;
    serde_json::to_value(&row).map_err(|e| AppError::Internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn fixture_ws(state: &AppState) -> String {
        repo::workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create ws")
            .id
    }

    #[tokio::test]
    async fn add_rejects_unknown_kind() {
        let state = AppState::for_tests().await;
        let ws = fixture_ws(&state).await;
        let err = add(
            &state,
            json!({ "workspaceId": ws, "title": "T", "kind": "bogus", "content": "x" }),
        )
        .await
        .expect_err("unknown kind must be rejected");
        assert!(matches!(err, AppError::Invalid(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn add_then_list_and_get_roundtrip() {
        let state = AppState::for_tests().await;
        let ws = fixture_ws(&state).await;
        let added = add(
            &state,
            json!({ "workspaceId": ws, "agentId": "ag-1", "title": "Doc", "kind": "markdown", "content": "# Hi" }),
        )
        .await
        .expect("add failed");
        let id = added.get("id").and_then(Value::as_str).expect("id").to_owned();

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        assert_eq!(listed.as_array().expect("array").len(), 1);

        let got = get(&state, json!({ "id": id })).await.expect("get failed");
        assert_eq!(got.get("kind").and_then(Value::as_str), Some("markdown"));
        assert_eq!(got.get("content").and_then(Value::as_str), Some("# Hi"));
    }

    #[tokio::test]
    async fn get_missing_is_not_found() {
        let state = AppState::for_tests().await;
        let err = get(&state, json!({ "id": "nope" })).await.expect_err("must 404");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn add_missing_workspace_is_not_found() {
        let state = AppState::for_tests().await;
        let err = add(
            &state,
            json!({ "workspaceId": "ghost", "title": "T", "kind": "text", "content": "x" }),
        )
        .await
        .expect_err("must 404");
        assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
    }
}

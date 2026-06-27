use crate::engine::repo::agent_definition::AgentDefinitionInput;
use crate::engine::repo::workspace_agent::WorkspaceAgentRow;
use crate::engine::{repo, AppError, AppState};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

// ── Request types ────────────────────────────────────────────────────────────

/// Payload for `agentDef.save` — create if `id` is absent, update if present.
///
/// `toolIds` / `skillIds` are accepted and forwarded without error but deferred
/// to M5 (when the `agent_tool` / `agent_skill` join tables will be persisted).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveAgentReq {
    id: Option<String>,
    name: String,
    #[serde(rename = "type")]
    agent_type: String,
    role: Option<String>,
    cli_kind: Option<String>,
    color: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
    harness_mode: Option<String>,
    share_blackboard: Option<bool>,
    auto_submit_injected: Option<bool>,
    allowed_senders: Option<String>,
    // Accepted but deferred — TODO(M5): persist agent_tool / agent_skill joins.
    #[allow(dead_code)]
    tool_ids: Option<Vec<String>>,
    #[allow(dead_code)]
    skill_ids: Option<Vec<String>>,
}

/// Payload for `agentDef.addToWorkspace`.
///
/// Adds one agent definition to one or more workspaces. For each workspace a
/// `workspace_agent` instance and its paired `session` are created atomically.
/// Already-linked (workspace, agentDef) pairs are silently reused (idempotent).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddToWorkspaceReq {
    agent_def_id: String,
    workspace_ids: Vec<String>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Return all agent definitions annotated with their workspace count.
///
/// Maps to `agentDef.list` on the IPC bus.
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let items = repo::agent_definition::list_with_counts(&state.db).await?;
    serde_json::to_value(items).map_err(|e| AppError::Internal(e.to_string()))
}

/// Create or update an agent definition.
///
/// Maps to `agentDef.save` on the IPC bus.
/// - `id` absent → INSERT (new UUID assigned).
/// - `id` present → UPDATE; `NotFound` error if the id doesn't exist.
pub async fn save(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SaveAgentReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let input = AgentDefinitionInput {
        name: req.name,
        role: req.role,
        agent_type: req.agent_type,
        cli_kind: req.cli_kind,
        color: req.color,
        provider_id: req.provider_id,
        model: req.model,
        harness_mode: req.harness_mode.unwrap_or_else(|| "own".to_owned()),
        share_blackboard: req.share_blackboard,
        auto_submit_injected: req.auto_submit_injected,
        allowed_senders: req.allowed_senders,
    };

    let row = match req.id.as_deref() {
        None => repo::agent_definition::create(&state.db, &input).await?,
        Some(id) => repo::agent_definition::update(&state.db, id, &input)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("agent_definition id={id} not found")))?,
    };

    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Link an agent definition to one or more workspaces.
///
/// Maps to `agentDef.addToWorkspace` on the IPC bus.
///
/// For each workspace_id:
/// 1. Validate the workspace and agent_definition both exist (`NotFound` if not).
/// 2. **Idempotency**: if the (workspace_id, agent_def_id) pair is already linked,
///    reuse the existing `workspace_agent` row — do not duplicate.
/// 3. Otherwise create a `workspace_agent` + `session` atomically in a SQLite
///    transaction so a failed session INSERT cannot leave an orphan instance.
///
/// # Transaction approach
///
/// chain-builder's `execute` accepts any `sqlx::Executor<'e>`, so it can run
/// against `&mut *tx` (`&mut SqliteConnection`). However, we use raw `sqlx::query`
/// for the two INSERTs inside the transaction — the same pattern `db::migrate` uses
/// — because it avoids lifetime complexity from chaining multiple QueryBuilders
/// through a single mutable borrow. The repo pool-based helpers (`workspace_agent::create`,
/// `session::create_for_instance`) are used by tests and read-only callers only.
///
/// The session's `context_limit` binds `session::DEFAULT_CONTEXT_LIMIT` directly,
/// so the const is the single source of truth.
pub async fn add_to_workspace(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: AddToWorkspaceReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    // Validate the (loop-invariant) agent definition once, up front. An empty
    // workspace_ids list returns [] — but still validate the def first so a bad
    // id is reported rather than silently no-op'd.
    if !repo::agent_definition::exists(&state.db, &req.agent_def_id).await? {
        return Err(AppError::NotFound(format!(
            "agent_definition id={} not found",
            req.agent_def_id
        )));
    }

    let mut results: Vec<WorkspaceAgentRow> = Vec::new();

    for workspace_id in &req.workspace_ids {
        // ── 1. Validate workspace exists ────────────────────────────────────
        if !repo::workspace::exists(&state.db, workspace_id).await? {
            return Err(AppError::NotFound(format!(
                "workspace id={workspace_id} not found"
            )));
        }

        // ── 2. Idempotency: reuse existing instance if already linked ────────
        if let Some(existing) =
            repo::workspace_agent::find(&state.db, workspace_id, &req.agent_def_id).await?
        {
            results.push(existing);
            continue;
        }

        // ── 3. Atomically create workspace_agent + session ───────────────────
        //
        // Raw sqlx inside a transaction — mirrors the pattern in db::migrate.
        let wa_id = Uuid::new_v4().to_string();
        let added_at = Utc::now().to_rfc3339();
        let session_id = Uuid::new_v4().to_string();
        let started_at = Utc::now().to_rfc3339();

        let mut tx = state.db.begin().await?;

        sqlx::query(
            "INSERT INTO workspace_agent \
             (id, workspace_id, agent_def_id, status, added_at) \
             VALUES (?, ?, ?, 'idle', ?)",
        )
        .bind(&wa_id)
        .bind(workspace_id.as_str())
        .bind(&req.agent_def_id)
        .bind(&added_at)
        .execute(&mut *tx)
        .await?;

        // Bind DEFAULT_CONTEXT_LIMIT (not a literal) so the const is the single
        // source of truth — no silent divergence if it ever changes.
        sqlx::query(
            "INSERT INTO session \
             (id, workspace_agent_id, context_tokens, context_limit, started_at, last_active_at) \
             VALUES (?, ?, 0, ?, ?, NULL)",
        )
        .bind(&session_id)
        .bind(&wa_id)
        .bind(repo::session::DEFAULT_CONTEXT_LIMIT)
        .bind(&started_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        results.push(WorkspaceAgentRow {
            id: wa_id,
            workspace_id: workspace_id.to_owned(),
            agent_def_id: req.agent_def_id.clone(),
            status: "idle".to_owned(),
            added_at,
        });
    }

    serde_json::to_value(&results).map_err(|e| AppError::Internal(e.to_string()))
}

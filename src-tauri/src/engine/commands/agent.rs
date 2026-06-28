use crate::engine::repo::agent_definition::AgentDefinitionInput;
use crate::engine::repo::workspace_agent::WorkspaceAgentRow;
use crate::engine::{repo, secrets, AppError, AppState};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Sentinel shown by the Builder for a secret env var that is already stored in
/// the Keychain; receiving it back on save means "keep the existing secret"
/// (must match `SECRET_PLACEHOLDER` in `src/components/Builder.tsx`).
const SECRET_PLACEHOLDER: &str = "••••••••";

/// Heuristic: does this env var NAME look like it holds a secret? Such values
/// are routed to the Keychain instead of the DB (constraint: secrets never land
/// in DB/logs/IPC). Matches AUTH_TOKEN / API_KEY / *_SECRET / *PASSWORD etc.
fn is_secret_env_key(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    k.contains("TOKEN")
        || k.contains("SECRET")
        || k.contains("PASSWORD")
        || k.contains("CREDENTIAL")
        || k.contains("KEY")
}

/// Keychain account for one agent's secret env var.
fn secret_account(agent_id: &str, env_key: &str) -> String {
    format!("agent_env:{agent_id}:{env_key}")
}

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
    // ── Claude Code / CLI launch config ──────────────────────────────────────
    permission_mode: Option<String>,
    custom_args: Option<String>,
    /// Full env map AS ENTERED (may contain secret values); the handler splits
    /// secret-looking keys out to the Keychain and stores only the rest in DB.
    custom_env: Option<BTreeMap<String, String>>,
    context_window: Option<String>,
    // Accepted but deferred — TODO(M5): persist agent_tool / agent_skill joins.
    #[allow(dead_code)]
    tool_ids: Option<Vec<String>>,
    #[allow(dead_code)]
    skill_ids: Option<Vec<String>>,
}

/// Payload for `agentDef.delete`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteAgentReq {
    id: String,
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

    // ── Split custom_env into non-secret (→ DB JSON) and secret (→ Keychain) ──
    // Secret VALUES never enter the DB; only their NAMES are recorded (so the
    // spawn path knows which to fetch back from the Keychain). On edit, a
    // secret value arriving as the placeholder/empty means "keep what's stored".
    let mut non_secret = serde_json::Map::new();
    let mut secret_names: Vec<String> = Vec::new();
    let mut secrets_to_write: Vec<(String, String)> = Vec::new();
    if let Some(env) = &req.custom_env {
        for (k, v) in env {
            if is_secret_env_key(k) {
                secret_names.push(k.clone());
                if !v.is_empty() && v != SECRET_PLACEHOLDER {
                    secrets_to_write.push((k.clone(), v.clone()));
                }
            } else {
                non_secret.insert(k.clone(), Value::String(v.clone()));
            }
        }
    }
    let custom_env = (!non_secret.is_empty()).then(|| Value::Object(non_secret).to_string());
    let secret_env_keys = (!secret_names.is_empty()).then(|| {
        serde_json::to_string(&secret_names).expect("serializing Vec<String> is infallible")
    });

    // Trim away blank optionals so they store as NULL, not "".
    let nonblank = |s: Option<String>| s.filter(|v| !v.trim().is_empty());

    // Validate permission_mode against a known allowlist: this value is
    // interpolated UNQUOTED into the `zsh -c` launch string, so an arbitrary
    // value with whitespace/metacharacters could alter the command. The set
    // mirrors Claude Code's `--permission-mode` choices.
    let permission_mode = nonblank(req.permission_mode);
    if let Some(mode) = permission_mode.as_deref() {
        const ALLOWED: [&str; 5] = [
            "default",
            "auto",
            "acceptEdits",
            "plan",
            "bypassPermissions",
        ];
        if !ALLOWED.contains(&mode) {
            return Err(AppError::Invalid(format!(
                "invalid permission mode: {mode}"
            )));
        }
    }

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
        permission_mode,
        custom_args: nonblank(req.custom_args),
        custom_env,
        secret_env_keys,
        context_window: nonblank(req.context_window),
    };

    // Capture the previously-stored secret key NAMES (UPDATE only) so we can
    // prune the Keychain entries the user removed on this edit.
    let old_secret_names: Vec<String> = match req.id.as_deref() {
        Some(id) => repo::agent_definition::get(&state.db, id)
            .await?
            .and_then(|r| r.secret_env_keys)
            .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
            .unwrap_or_default(),
        None => Vec::new(),
    };

    let row = match req.id.as_deref() {
        None => repo::agent_definition::create(&state.db, &input).await?,
        Some(id) => repo::agent_definition::update(&state.db, id, &input)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("agent_definition id={id} not found")))?,
    };

    // Persist the secret env values to the Keychain, keyed by the (now-known)
    // agent id. Done AFTER the upsert so a create has its generated id.
    for (name, value) in &secrets_to_write {
        secrets::set_key(&secret_account(&row.id, name), value)
            .map_err(|e| AppError::Internal(format!("store secret env {name}: {e}")))?;
    }
    // Prune Keychain entries for secret keys that are no longer present so a
    // removed credential doesn't linger. delete_key is idempotent on a miss.
    for name in &old_secret_names {
        if !secret_names.contains(name) {
            let _ = secrets::delete_key(&secret_account(&row.id, name));
        }
    }

    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Delete an agent definition everywhere.
///
/// Maps to `agentDef.delete` on the IPC bus. Removes every `workspace_agent`
/// instance of this definition first (FK-safe + live-backend teardown) so the
/// definition's `NO ACTION` foreign key can't abort the delete, then deletes the
/// definition (its `agent_tool` / `agent_skill` / `fusion_config` rows cascade)
/// and drops its secret env values from the Keychain.
pub async fn delete(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DeleteAgentReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let id = req.id;

    let def = repo::agent_definition::get(&state.db, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("agent_definition id={id} not found")))?;

    // Remove every instance of this def across all workspaces.
    let instances = repo::workspace_agent::list_by_agent_def(&state.db, &id).await?;
    for inst in &instances {
        // Tear down any live backend before deleting the rows.
        let _ = state.runtime.unregister(&inst.id);
        repo::workspace_agent::remove(&state.db, &inst.id).await?;
    }

    repo::agent_definition::delete(&state.db, &id).await?;

    // Drop this def's secret env values from the Keychain (best-effort).
    if let Some(text) = def.secret_env_keys {
        if let Ok(names) = serde_json::from_str::<Vec<String>>(&text) {
            for name in names {
                let _ = secrets::delete_key(&secret_account(&id, &name));
            }
        }
    }

    Ok(Value::Null)
}

/// Link an agent definition to one or more workspaces.
///
/// Maps to `agentDef.addToWorkspace` on the IPC bus.
///
/// For each workspace_id:
/// 1. Validate the workspace exists (`NotFound` if not).
/// 2. Call `repo::workspace_agent::instantiate` — the shared helper that
///    idempotently finds-or-creates the `workspace_agent` + `session` pair
///    in a single SQLite transaction (no orphan risk, no duplicate rows).
///
/// The agent_definition is validated once before the loop so a bad id is
/// reported rather than silently no-op'd on an empty workspace_ids list.
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

        // ── 2 & 3. Idempotent create workspace_agent + session (atomic) ──────
        //
        // `instantiate` is the single source of truth for the find-or-create
        // transaction — no raw sqlx duplicated here.
        let row =
            repo::workspace_agent::instantiate(&state.db, workspace_id, &req.agent_def_id).await?;
        results.push(row);
    }

    serde_json::to_value(&results).map_err(|e| AppError::Internal(e.to_string()))
}

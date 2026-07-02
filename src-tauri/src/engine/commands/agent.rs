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
/// `toolIds` is accepted and forwarded without error but deferred (agent_tool
/// wiring is out of scope for the v1 skill system). `skillIds` IS persisted —
/// see the `set_custom_attachments` call in `save()`.
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
    // Same basis as the launch snapshot (`repo::skill::content_for_agent`):
    // the EFFECTIVE builtin set (mandatory + this item's selected optional
    // ones, via `effective_from` — see ADR 0003) first, then custom ids — so
    // `AgentDefinition.skillIds` and `WorkspaceAgent.launchedSkillIds` are
    // directly comparable (see Roster.tsx's `computeSkillsStale`).
    let skill_map = repo::skill::custom_skill_ids_by_agent(&state.db).await?;
    // Fetched ONCE here (a filesystem scan of the skills folder), then
    // filtered per item below via `effective_from` — not `list_builtin()`
    // per item, which would turn one `agentDef.list` call into N filesystem
    // scans (one per agent definition returned above).
    let builtins = repo::skill::list_builtin();

    let mut value = serde_json::to_value(&items).map_err(|e| AppError::Internal(e.to_string()))?;
    if let Some(arr) = value.as_array_mut() {
        for item in arr.iter_mut() {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let selected_optional: Vec<String> = item
                .get("selectedBuiltinSkillIds")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let mut ids: Vec<String> = repo::skill::effective_from(&builtins, &selected_optional)
                .into_iter()
                .map(|s| s.id)
                .collect();
            if let Some(custom_ids) = skill_map.get(&id) {
                ids.extend(custom_ids.iter().cloned());
            }
            if !ids.is_empty() {
                item["skillIds"] = serde_json::json!(ids);
            }
        }
    }
    Ok(value)
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

    // Split the incoming skillIds into three groups: a real custom DB skill
    // id -> agent_skill (unchanged path); an OPTIONAL builtin's id (see ADR
    // 0003) -> agent_definition.selected_builtin_skill_ids; anything else (a
    // mandatory builtin id, an unknown/stale id) -> silently dropped, same
    // "filter to known ids" precedent as the pre-existing custom-only path.
    let requested_skill_ids = req.skill_ids.unwrap_or_default();
    let optional_builtin_ids: std::collections::HashSet<String> = repo::skill::list_builtin()
        .into_iter()
        .filter(|s| !s.mandatory)
        .map(|s| s.id)
        .collect();
    let valid_custom_ids: std::collections::HashSet<String> = repo::skill::list(&state.db)
        .await?
        .into_iter()
        .map(|s| s.id)
        .collect();
    let filtered_custom_skill_ids: Vec<String> = requested_skill_ids
        .iter()
        .filter(|id| valid_custom_ids.contains(*id))
        .cloned()
        .collect();
    let filtered_optional_builtin_ids: Vec<String> = requested_skill_ids
        .iter()
        .filter(|id| optional_builtin_ids.contains(*id))
        .cloned()
        .collect();
    let selected_builtin_skill_ids = (!filtered_optional_builtin_ids.is_empty()).then(|| {
        serde_json::to_string(&filtered_optional_builtin_ids)
            .expect("serializing Vec<String> is infallible")
    });

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
        selected_builtin_skill_ids,
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

    // Persist custom skill attachments (replace semantics) — the filtered
    // set was already computed above, before `row` existed, so both this and
    // the optional-builtin selection (already stored via `input` in the
    // create/update call above) come from one shared split of `skillIds`.
    repo::skill::set_custom_attachments(&state.db, &row.id, &filtered_custom_skill_ids).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_persists_and_replaces_skill_attachments() {
        let state = AppState::for_tests().await;
        let skill = repo::skill::create(&state.db, "S1", None, "c")
            .await
            .expect("create skill failed");

        let created = save(
            &state,
            serde_json::json!({
                "name": "Atlas", "type": "cli", "harnessMode": "own",
                "skillIds": [skill.id],
            }),
        )
        .await
        .expect("create agent failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let attached = repo::skill::attached_to_agent(&state.db, &id)
            .await
            .expect("query failed");
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].id, skill.id);

        // A second save with an EMPTY skillIds list must clear the attachment
        // (replace semantics, not merge).
        save(
            &state,
            serde_json::json!({
                "id": id, "name": "Atlas", "type": "cli", "harnessMode": "own",
                "skillIds": [],
            }),
        )
        .await
        .expect("update agent failed");
        let attached_after = repo::skill::attached_to_agent(&state.db, &id)
            .await
            .expect("query failed");
        assert!(attached_after.is_empty());
    }

    #[tokio::test]
    async fn save_silently_drops_unknown_or_builtin_skill_ids() {
        let _fx = repo::skill::test_support::fixture_skills_dir("cmd-agent-save-drops");
        let state = AppState::for_tests().await;

        let created = save(
            &state,
            serde_json::json!({
                "name": "Atlas", "type": "cli", "harnessMode": "own",
                "skillIds": ["fix-mandatory", "no-such-id"],
            }),
        )
        .await
        .expect("create failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let attached = repo::skill::attached_to_agent(&state.db, &id)
            .await
            .expect("query failed");
        assert!(
            attached.is_empty(),
            "builtin/unknown ids must never become agent_skill rows"
        );
    }

    #[tokio::test]
    async fn save_splits_skill_ids_into_custom_and_optional_builtin() {
        let _fx = repo::skill::test_support::fixture_skills_dir("cmd-agent-save-splits");
        let state = AppState::for_tests().await;
        let custom = repo::skill::create(&state.db, "Custom", None, "c")
            .await
            .expect("create skill failed");

        let created = save(
            &state,
            serde_json::json!({
                "name": "A",
                "type": "cli",
                "skillIds": [custom.id, "fix-optional", "fix-mandatory", "no-such-id"],
            }),
        )
        .await
        .expect("save failed");
        let id = created["id"].as_str().unwrap().to_owned();

        // Custom id -> agent_skill.
        let attached = repo::skill::attached_to_agent(&state.db, &id)
            .await
            .expect("query failed");
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].id, custom.id);

        // Optional builtin id -> agent_definition.selected_builtin_skill_ids;
        // the mandatory "fix-mandatory" id and the unknown id are both dropped.
        let def = repo::agent_definition::get(&state.db, &id)
            .await
            .expect("get failed")
            .expect("row should exist");
        let selected: Vec<String> = def
            .selected_builtin_skill_ids
            .as_deref()
            .map(|t| serde_json::from_str(t).expect("valid json"))
            .unwrap_or_default();
        assert_eq!(selected, vec!["fix-optional".to_string()]);
    }

    #[tokio::test]
    async fn list_annotates_skill_ids() {
        let _fx = repo::skill::test_support::fixture_skills_dir("cmd-agent-list-annotates");
        let state = AppState::for_tests().await;
        let skill = repo::skill::create(&state.db, "S1", None, "c")
            .await
            .expect("create failed");
        let created = save(
            &state,
            serde_json::json!({
                "name": "Atlas", "type": "cli", "harnessMode": "own",
                "skillIds": [skill.id],
            }),
        )
        .await
        .expect("create failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let listed = list(&state, Value::Null).await.expect("list failed");
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .unwrap();
        assert_eq!(item["skillIds"].as_array().map(|a| a.len()), Some(2));
    }

    /// A definition with ZERO custom skill attachments must still get a
    /// `skillIds` entry for a builtin skill row — it's never attached via
    /// `agentDef.save`, but `content_for_agent` (the launch snapshot) always
    /// includes it, so `list()`'s annotation must too (Roster staleness fix).
    #[tokio::test]
    async fn list_annotates_builtin_skill_ids_even_without_attachment() {
        let _fx =
            repo::skill::test_support::fixture_skills_dir("cmd-agent-list-builtin-unattached");
        let state = AppState::for_tests().await;

        let created = save(
            &state,
            serde_json::json!({
                "name": "Atlas", "type": "cli", "harnessMode": "own",
            }),
        )
        .await
        .expect("create failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let listed = list(&state, Value::Null).await.expect("list failed");
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .unwrap();
        let ids: Vec<String> = item["skillIds"]
            .as_array()
            .expect("skillIds must be present even with zero custom attachments")
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert!(
            ids.contains(&"fix-mandatory".to_string()),
            "the mandatory fixture builtin must appear even though nothing was attached: {ids:?}"
        );
    }

    #[tokio::test]
    async fn list_skill_ids_include_selected_optional_builtin_but_exclude_unselected() {
        let _fx = repo::skill::test_support::fixture_skills_dir("cmd-agent-list-optional-selected");
        let state = AppState::for_tests().await;
        let created = save(
            &state,
            serde_json::json!({
                "name": "A",
                "type": "cli",
                "skillIds": ["fix-optional"],
            }),
        )
        .await
        .expect("save failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let listed = list(&state, Value::Null).await.expect("list failed");
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .expect("item present");
        let ids: Vec<String> = item["skillIds"]
            .as_array()
            .expect("skillIds must be present")
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert!(
            ids.contains(&"fix-mandatory".to_string()),
            "mandatory builtin always present"
        );
        assert!(
            ids.contains(&"fix-optional".to_string()),
            "selected optional builtin must be present"
        );
    }

    #[tokio::test]
    async fn list_skill_ids_exclude_optional_builtin_when_not_selected() {
        let _fx =
            repo::skill::test_support::fixture_skills_dir("cmd-agent-list-optional-unselected");
        let state = AppState::for_tests().await;
        let created = save(&state, serde_json::json!({ "name": "A", "type": "cli" }))
            .await
            .expect("save failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let listed = list(&state, Value::Null).await.expect("list failed");
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .expect("item present");
        let ids: Vec<String> = item["skillIds"]
            .as_array()
            .expect("skillIds must be present (mandatory builtin still applies)")
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert!(ids.contains(&"fix-mandatory".to_string()));
        assert!(!ids.contains(&"fix-optional".to_string()));
    }

    #[tokio::test]
    async fn list_skill_ids_are_isolated_per_agent_definition() {
        let _fx = repo::skill::test_support::fixture_skills_dir("cmd-agent-list-isolated");
        let state = AppState::for_tests().await;
        let created_a = save(
            &state,
            serde_json::json!({
                "name": "A",
                "type": "cli",
                "skillIds": ["fix-optional"],
            }),
        )
        .await
        .expect("save failed");
        let id_a = created_a["id"].as_str().unwrap().to_owned();

        let created_b = save(&state, serde_json::json!({ "name": "B", "type": "cli" }))
            .await
            .expect("save failed");
        let id_b = created_b["id"].as_str().unwrap().to_owned();

        let listed = list(&state, Value::Null).await.expect("list failed");
        let listed = listed.as_array().unwrap();

        let item_a = listed
            .iter()
            .find(|i| i["id"] == id_a)
            .expect("item A present");
        let ids_a: Vec<String> = item_a["skillIds"]
            .as_array()
            .expect("skillIds must be present")
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert!(
            ids_a.contains(&"fix-mandatory".to_string()),
            "mandatory builtin always present"
        );
        assert!(
            ids_a.contains(&"fix-optional".to_string()),
            "agent A's selected optional builtin must be present"
        );

        let item_b = listed
            .iter()
            .find(|i| i["id"] == id_b)
            .expect("item B present");
        let ids_b: Vec<String> = item_b["skillIds"]
            .as_array()
            .expect("skillIds must be present (mandatory builtin still applies)")
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert!(ids_b.contains(&"fix-mandatory".to_string()));
        assert!(
            !ids_b.contains(&"fix-optional".to_string()),
            "agent B must not inherit agent A's optional builtin selection"
        );
    }
}

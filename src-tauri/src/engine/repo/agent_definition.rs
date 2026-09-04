//! AgentDefinition repository — mirrors `workspace.rs` pattern exactly.
//!
//! # chain-builder usage
//!
//! - `list`, `get`: chain-builder SELECT with `fetch_all` / `fetch_optional`.
//! - `create`: chain-builder INSERT (all binds cast to `Value`).
//! - `update`: chain-builder UPDATE with `where_eq("id", …)` and `execute`.
//!
//! # list_with_counts
//!
//! Uses raw `sqlx::query_as` because the subquery
//! `(SELECT COUNT(*) … WHERE wa.agent_def_id = d.id)` is awkward to express in
//! chain-builder's current API. The extra `in_workspaces` column is decoded into
//! [`AgentDefListItem`] via its own `sqlx::FromRow` impl.
//!
//! # bool / nullable handling
//!
//! `share_blackboard` and `auto_submit_injected` are `INTEGER` (nullable) in the
//! schema, so they decode as `Option<bool>` — sqlx SQLite maps non-NULL INTEGER
//! to `bool` (0 = false, non-zero = true) and NULL to `None`. On the INSERT /
//! UPDATE side `Value::Bool(b)` is supported by chain-builder's SQLite dialect.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite, Value as Bind};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Serialize a column that stores JSON text (an object or array) by parsing it
/// back into structured JSON, so the IPC payload carries `customEnv` as an
/// object and `secretEnvKeys` as an array instead of as an opaque string. A
/// malformed value degrades to `null` rather than failing the whole response.
fn serialize_json_text<S>(opt: &Option<String>, ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match opt {
        Some(text) => serde_json::from_str::<serde_json::Value>(text)
            .unwrap_or(serde_json::Value::Null)
            .serialize(ser),
        None => ser.serialize_none(),
    }
}

// ── Row structs ─────────────────────────────────────────────────────────────

/// Decoded row from the `agent_definition` table.
///
/// `sqlx::FromRow` maps snake_case column names to snake_case fields.
/// `serde(rename_all = "camelCase")` then emits camelCase JSON that matches the
/// `AgentDefinition` interface in `src/ipc/types.ts`.
/// `skip_serializing_if` on optional columns omits the key entirely when `None`
/// (matches `field?: T`, not `field: T | null`).
///
/// The Rust keyword `type` must be written `r#type`; serde's `rename_all`
/// strips the raw-identifier prefix so the JSON key is plain `"type"`.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefRow {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The chosen first-class role id (builtin slug or custom `role.id`) — see
    /// ADR 0005. `None` for pre-role-system agents; the legacy `role` free-text
    /// column above is the display fallback for those.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Antigravity reasoning effort. `None` means Auto/omit; non-Antigravity
    /// definitions are normalized to `None` on every write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub harness_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_blackboard: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_submit_injected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_senders: Option<String>,
    // ── Claude Code / CLI launch config (M-CLI-config) ───────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_args: Option<String>,
    /// JSON object of NON-secret env vars; serialized back to an object.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_json_text"
    )]
    pub custom_env: Option<String>,
    /// JSON array of env var NAMES whose values live in the Keychain.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_json_text"
    )]
    pub secret_env_keys: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<String>,
    /// JSON array of OPTIONAL builtin skill ids (`mandatory: false`) this
    /// definition has opted into (see ADR 0003). `None`/absent means no
    /// optional builtins selected — distinct from an empty JSON array only
    /// in that both mean the same thing here (no meaningful distinction is
    /// drawn between "never set" and "explicitly cleared").
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_json_text"
    )]
    pub selected_builtin_skill_ids: Option<String>,
    /// rtk (Claude Code hook) toggle. `None` OR `Some(true)` = enabled;
    /// `Some(false)` = disabled. Nullable INTEGER column, NULL defaults ON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtk_enabled: Option<bool>,
    pub created_at: String,
}

/// Like [`AgentDefRow`] but with an extra `in_workspaces` annotation added by
/// [`list_with_counts`] (mirrors `AgentDefinition.inWorkspaces` in the TS types).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefListItem {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub harness_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_blackboard: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_submit_injected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_senders: Option<String>,
    // ── Claude Code / CLI launch config (M-CLI-config) ───────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_args: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_json_text"
    )]
    pub custom_env: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_json_text"
    )]
    pub secret_env_keys: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_json_text"
    )]
    pub selected_builtin_skill_ids: Option<String>,
    /// rtk (Claude Code hook) toggle. `None` OR `Some(true)` = enabled;
    /// `Some(false)` = disabled. Nullable INTEGER column, NULL defaults ON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtk_enabled: Option<bool>,
    pub created_at: String,
    /// How many workspaces this definition has been added to.
    pub in_workspaces: i64,
}

// ── Column list (shared between list and get) ────────────────────────────────

const COLS: [&str; 23] = [
    "id",
    "name",
    "role",
    "role_id",
    "type",
    "cli_kind",
    "color",
    "default_level",
    "provider_id",
    "model",
    "effort",
    "harness_mode",
    "share_blackboard",
    "auto_submit_injected",
    "allowed_senders",
    "permission_mode",
    "custom_args",
    "custom_env",
    "secret_env_keys",
    "context_window",
    "selected_builtin_skill_ids",
    "rtk_enabled",
    "created_at",
];

/// Named-field input for [`create`] / [`update`].
///
/// Bundles the mutable columns of an agent definition so call sites pass a
/// single self-documenting struct instead of a long positional argument list.
/// `id` and `created_at` are NOT part of this struct — `create` generates them,
/// and `update` preserves `created_at` and takes `id` separately.
#[derive(Debug, Clone, Default)]
pub struct AgentDefinitionInput {
    pub name: String,
    pub role: Option<String>,
    /// The chosen first-class role id (builtin slug or custom `role.id`), ADR
    /// 0005. `None` leaves the agent role-less (the `role` free-text above may
    /// still carry a legacy display label).
    pub role_id: Option<String>,
    pub agent_type: String,
    pub cli_kind: Option<String>,
    pub color: Option<String>,
    pub default_level: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    /// Antigravity reasoning effort (`low` / `medium` / `high`). `None` means
    /// Auto and is also the normalized value for every other CLI harness.
    pub effort: Option<String>,
    pub harness_mode: String,
    pub share_blackboard: Option<bool>,
    pub auto_submit_injected: Option<bool>,
    pub allowed_senders: Option<String>,
    /// `--permission-mode` value (e.g. "auto" / "bypassPermissions").
    pub permission_mode: Option<String>,
    /// Extra CLI args appended verbatim to the launch command.
    pub custom_args: Option<String>,
    /// JSON object of NON-secret env vars (secrets are split to the Keychain).
    pub custom_env: Option<String>,
    /// JSON array of env var NAMES whose values are stored in the Keychain.
    pub secret_env_keys: Option<String>,
    /// "1m" / "200k" — selects the model's context window.
    pub context_window: Option<String>,
    /// JSON array of optional builtin skill ids selected for this agent
    /// definition (see ADR 0003). `None` clears the selection.
    pub selected_builtin_skill_ids: Option<String>,
    /// rtk (Claude Code hook) toggle. `None` OR `Some(true)` = enabled;
    /// `Some(false)` = disabled.
    pub rtk_enabled: Option<bool>,
}

// ── CRUD ─────────────────────────────────────────────────────────────────────

/// Return all agent definitions ordered by `created_at` ascending, `id` as
/// stable tie-breaker.
///
/// Used by M1.3 (Agent Library screen). Suppressed until then.
#[allow(dead_code)]
pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<AgentDefRow>> {
    QueryBuilder::<Sqlite>::table("agent_definition")
        .select(COLS)
        .order_by("created_at", Order::Asc)
        .order_by("id", Order::Asc)
        .fetch_all::<AgentDefRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Return all definitions annotated with `in_workspaces` count.
///
/// Uses raw sqlx because the correlated subquery
/// `(SELECT COUNT(*) FROM workspace_agent WHERE agent_def_id = d.id)`
/// is awkward to express via chain-builder's current API.
pub async fn list_with_counts(pool: &SqlitePool) -> sqlx::Result<Vec<AgentDefListItem>> {
    // NOTE: this hardcoded column list must stay in sync with `COLS` (plus the
    // appended `in_workspaces` count) if the table's columns ever change.
    sqlx::query_as::<_, AgentDefListItem>(
        "SELECT d.id, d.name, d.role, d.role_id, d.type, d.cli_kind, d.color, d.default_level, \
         d.provider_id, d.model, d.effort, \
         d.harness_mode, d.share_blackboard, d.auto_submit_injected, d.allowed_senders, \
         d.permission_mode, d.custom_args, d.custom_env, d.secret_env_keys, d.context_window, \
         d.selected_builtin_skill_ids, d.rtk_enabled, \
         d.created_at, \
         (SELECT COUNT(*) FROM workspace_agent wa WHERE wa.agent_def_id = d.id) AS in_workspaces \
         FROM agent_definition d \
         ORDER BY d.created_at, d.id",
    )
    .fetch_all(pool)
    .await
}

/// Fetch a single definition by `id`, or `None` if it does not exist.
pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<AgentDefRow>> {
    QueryBuilder::<Sqlite>::table("agent_definition")
        .select(COLS)
        .where_eq("id", id)
        .fetch_optional::<AgentDefRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Return `true` if an agent definition with `id` exists.
///
/// Delegates to `get()` (mirrors `workspace::exists`) — avoids a separate COUNT
/// query and extra trait bounds. Called by `agentDef.addToWorkspace` (M1.5).
pub async fn exists(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    get(pool, id).await.map(|opt| opt.is_some())
}

/// Create a new agent definition from `input` and return the constructed row.
///
/// Generates a UUID v4 `id` and ISO-8601 UTC `created_at` timestamp.
/// All INSERT bind values are `Value` so the array is homogeneous:
/// optional `Text` fields become `Bind::Text(s)` or `Bind::Null` without a
/// separate raw-sqlx path. `Option<bool>` bool fields use `Bind::Bool` / `Bind::Null`.
pub async fn create(pool: &SqlitePool, input: &AgentDefinitionInput) -> sqlx::Result<AgentDefRow> {
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let mut input = input.clone();
    if input.cli_kind.as_deref() != Some("antigravity") {
        input.effort = None;
    }

    QueryBuilder::<Sqlite>::table("agent_definition")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("name", Bind::Text(input.name.clone())),
            (
                "role",
                input.role.clone().map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "role_id",
                input.role_id.clone().map(Bind::Text).unwrap_or(Bind::Null),
            ),
            ("type", Bind::Text(input.agent_type.clone())),
            (
                "cli_kind",
                input.cli_kind.clone().map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "color",
                input.color.clone().map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "default_level",
                input
                    .default_level
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "provider_id",
                input
                    .provider_id
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "model",
                input.model.clone().map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "effort",
                input.effort.clone().map(Bind::Text).unwrap_or(Bind::Null),
            ),
            ("harness_mode", Bind::Text(input.harness_mode.clone())),
            (
                "share_blackboard",
                input.share_blackboard.map(Bind::Bool).unwrap_or(Bind::Null),
            ),
            (
                "auto_submit_injected",
                input
                    .auto_submit_injected
                    .map(Bind::Bool)
                    .unwrap_or(Bind::Null),
            ),
            (
                "allowed_senders",
                input
                    .allowed_senders
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "permission_mode",
                input
                    .permission_mode
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "custom_args",
                input
                    .custom_args
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "custom_env",
                input
                    .custom_env
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "secret_env_keys",
                input
                    .secret_env_keys
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "context_window",
                input
                    .context_window
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "selected_builtin_skill_ids",
                input
                    .selected_builtin_skill_ids
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "rtk_enabled",
                input.rtk_enabled.map(Bind::Bool).unwrap_or(Bind::Null),
            ),
            ("created_at", Bind::Text(created_at.clone())),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(AgentDefRow {
        id,
        name: input.name,
        role: input.role,
        role_id: input.role_id,
        r#type: input.agent_type,
        cli_kind: input.cli_kind,
        color: input.color,
        default_level: input.default_level,
        provider_id: input.provider_id,
        model: input.model,
        effort: input.effort,
        harness_mode: input.harness_mode,
        share_blackboard: input.share_blackboard,
        auto_submit_injected: input.auto_submit_injected,
        allowed_senders: input.allowed_senders,
        permission_mode: input.permission_mode,
        custom_args: input.custom_args,
        custom_env: input.custom_env,
        secret_env_keys: input.secret_env_keys,
        context_window: input.context_window,
        selected_builtin_skill_ids: input.selected_builtin_skill_ids,
        rtk_enabled: input.rtk_enabled,
        created_at,
    })
}

/// Update an existing agent definition's mutable fields and return the updated row.
///
/// Uses chain-builder UPDATE. Returns `None` if no row with `id` exists.
/// `created_at` is intentionally excluded — it is immutable after creation.
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    input: &AgentDefinitionInput,
) -> sqlx::Result<Option<AgentDefRow>> {
    let mut input = input.clone();
    if input.cli_kind.as_deref() != Some("antigravity") {
        input.effort = None;
    }

    QueryBuilder::<Sqlite>::table("agent_definition")
        .update([
            ("name", Bind::Text(input.name)),
            ("role", input.role.map(Bind::Text).unwrap_or(Bind::Null)),
            (
                "role_id",
                input.role_id.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            ("type", Bind::Text(input.agent_type)),
            (
                "cli_kind",
                input.cli_kind.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            ("color", input.color.map(Bind::Text).unwrap_or(Bind::Null)),
            (
                "default_level",
                input.default_level.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "provider_id",
                input.provider_id.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            ("model", input.model.map(Bind::Text).unwrap_or(Bind::Null)),
            ("effort", input.effort.map(Bind::Text).unwrap_or(Bind::Null)),
            ("harness_mode", Bind::Text(input.harness_mode)),
            (
                "share_blackboard",
                input.share_blackboard.map(Bind::Bool).unwrap_or(Bind::Null),
            ),
            (
                "auto_submit_injected",
                input
                    .auto_submit_injected
                    .map(Bind::Bool)
                    .unwrap_or(Bind::Null),
            ),
            (
                "allowed_senders",
                input.allowed_senders.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "permission_mode",
                input.permission_mode.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "custom_args",
                input.custom_args.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "custom_env",
                input.custom_env.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "secret_env_keys",
                input.secret_env_keys.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "context_window",
                input.context_window.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "selected_builtin_skill_ids",
                input
                    .selected_builtin_skill_ids
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "rtk_enabled",
                input.rtk_enabled.map(Bind::Bool).unwrap_or(Bind::Null),
            ),
        ])
        .where_eq("id", id)
        .execute(pool)
        .await
        .map_err(cb_err)?;

    get(pool, id).await
}

/// Delete an agent definition. Returns `true` if a row was deleted.
///
/// The caller MUST first remove the definition's `workspace_agent` instances —
/// `workspace_agent.agent_def_id` is `NOT NULL` with the default `NO ACTION`,
/// so this DELETE aborts while any instance exists. The `agent_tool`,
/// `agent_skill`, and `fusion_config` rows that reference the definition cascade
/// away automatically. Raw sqlx (not chain-builder) for a plain DELETE-by-id.
pub async fn delete(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    let res = sqlx::query("DELETE FROM agent_definition WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    /// Build a minimal input (only the required NOT NULL fields set).
    fn minimal_input(name: &str, agent_type: &str, harness_mode: &str) -> AgentDefinitionInput {
        AgentDefinitionInput {
            name: name.to_owned(),
            role: None,
            role_id: None,
            agent_type: agent_type.to_owned(),
            cli_kind: None,
            color: None,
            default_level: None,
            provider_id: None,
            model: None,
            effort: None,
            harness_mode: harness_mode.to_owned(),
            share_blackboard: None,
            auto_submit_injected: None,
            allowed_senders: None,
            permission_mode: None,
            custom_args: None,
            custom_env: None,
            secret_env_keys: None,
            context_window: None,
            selected_builtin_skill_ids: None,
            rtk_enabled: None,
        }
    }

    /// Create an agent definition and verify every field round-trips through the DB.
    /// Covers nullable fields (Some/None) and bool fields (true/false/None).
    #[tokio::test]
    async fn create_then_get_roundtrip() {
        let pool = connect_in_memory().await;

        // Full create with all optional fields populated.
        let row = create(
            &pool,
            &AgentDefinitionInput {
                name: "Atlas".into(),
                role: Some("Code runner".into()),
                role_id: None,
                agent_type: "cli".into(),
                cli_kind: Some("claude-code".into()),
                color: Some("#ff7a45".into()),
                default_level: Some("senior".into()),
                provider_id: None,
                model: Some("claude-opus-4-8".into()),
                effort: None,
                harness_mode: "own".into(),
                share_blackboard: Some(true),
                auto_submit_injected: Some(false),
                allowed_senders: Some("all".into()),
                permission_mode: Some("bypassPermissions".into()),
                custom_args: Some("--verbose".into()),
                custom_env: Some(r#"{"ANTHROPIC_BASE_URL":"https://openrouter.ai/api"}"#.into()),
                secret_env_keys: Some(r#"["ANTHROPIC_AUTH_TOKEN"]"#.into()),
                context_window: Some("1m".into()),
                selected_builtin_skill_ids: None,
                rtk_enabled: Some(false),
            },
        )
        .await
        .expect("create with all fields failed");

        assert_eq!(row.name, "Atlas");
        assert_eq!(row.permission_mode.as_deref(), Some("bypassPermissions"));
        assert_eq!(row.custom_args.as_deref(), Some("--verbose"));
        assert_eq!(row.context_window.as_deref(), Some("1m"));
        assert_eq!(
            row.custom_env.as_deref(),
            Some(r#"{"ANTHROPIC_BASE_URL":"https://openrouter.ai/api"}"#)
        );
        assert_eq!(
            row.secret_env_keys.as_deref(),
            Some(r#"["ANTHROPIC_AUTH_TOKEN"]"#)
        );
        assert_eq!(row.r#type, "cli");
        assert_eq!(row.harness_mode, "own");
        assert_eq!(row.role.as_deref(), Some("Code runner"));
        assert_eq!(row.cli_kind.as_deref(), Some("claude-code"));
        assert_eq!(row.color.as_deref(), Some("#ff7a45"));
        assert_eq!(row.default_level.as_deref(), Some("senior"));
        assert!(row.provider_id.is_none());
        assert_eq!(row.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(row.share_blackboard, Some(true));
        assert_eq!(row.auto_submit_injected, Some(false));
        assert_eq!(row.allowed_senders.as_deref(), Some("all"));
        assert_eq!(row.rtk_enabled, Some(false), "explicit disable round-trips");
        assert!(!row.id.is_empty());
        assert!(!row.created_at.is_empty());

        let fetched = get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched, row);

        // Nullable fields round-trip to None.
        let minimal = create(&pool, &minimal_input("Iris", "chat", "central"))
            .await
            .expect("minimal create failed");

        assert!(minimal.role.is_none());
        assert!(minimal.cli_kind.is_none());
        assert!(minimal.color.is_none());
        assert!(minimal.share_blackboard.is_none());
        assert!(minimal.auto_submit_injected.is_none());
        assert!(minimal.allowed_senders.is_none());
        assert!(minimal.permission_mode.is_none());
        assert!(minimal.custom_args.is_none());
        assert!(minimal.custom_env.is_none());
        assert!(minimal.secret_env_keys.is_none());
        assert!(minimal.context_window.is_none());
        assert!(
            minimal.rtk_enabled.is_none(),
            "unset rtk_enabled round-trips to None (means enabled)"
        );

        let fetched2 = get(&pool, &minimal.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched2, minimal);
    }

    /// update() changes fields; created_at is preserved.
    #[tokio::test]
    async fn update_changes_fields() {
        let pool = connect_in_memory().await;

        let row = create(&pool, &minimal_input("Nova", "chat", "own"))
            .await
            .expect("create failed");

        let updated = update(
            &pool,
            &row.id,
            &AgentDefinitionInput {
                name: "Nova-v2".into(),
                role: Some("Planner".into()),
                role_id: None,
                agent_type: "orchestrator".into(),
                cli_kind: None,
                color: Some("#5e5ce6".into()),
                default_level: Some("principal".into()),
                provider_id: None,
                model: Some("claude-opus-4-8".into()),
                effort: None,
                harness_mode: "central".into(),
                share_blackboard: Some(true),
                auto_submit_injected: Some(true),
                allowed_senders: Some("selected".into()),
                permission_mode: Some("auto".into()),
                custom_args: None,
                custom_env: Some(r#"{"ANTHROPIC_MODEL":"gpt-5.5"}"#.into()),
                secret_env_keys: None,
                context_window: Some("200k".into()),
                selected_builtin_skill_ids: None,
                rtk_enabled: Some(true),
            },
        )
        .await
        .expect("update failed")
        .expect("row should exist after update");

        assert_eq!(updated.name, "Nova-v2");
        assert_eq!(updated.permission_mode.as_deref(), Some("auto"));
        assert_eq!(updated.context_window.as_deref(), Some("200k"));
        assert_eq!(
            updated.custom_env.as_deref(),
            Some(r#"{"ANTHROPIC_MODEL":"gpt-5.5"}"#)
        );
        assert_eq!(updated.r#type, "orchestrator");
        assert_eq!(updated.harness_mode, "central");
        assert_eq!(updated.role.as_deref(), Some("Planner"));
        assert_eq!(updated.color.as_deref(), Some("#5e5ce6"));
        assert_eq!(updated.default_level.as_deref(), Some("principal"));
        assert_eq!(updated.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(updated.share_blackboard, Some(true));
        assert_eq!(updated.auto_submit_injected, Some(true));
        assert_eq!(updated.allowed_senders.as_deref(), Some("selected"));
        assert_eq!(
            updated.rtk_enabled,
            Some(true),
            "explicit enable round-trips through update"
        );
        // created_at preserved
        assert_eq!(updated.created_at, row.created_at);
    }

    /// delete() removes the row and returns true; deleting again returns false.
    #[tokio::test]
    async fn delete_removes_definition() {
        let pool = connect_in_memory().await;
        let row = create(&pool, &minimal_input("Gone", "cli", "own"))
            .await
            .expect("create failed");

        assert!(delete(&pool, &row.id).await.expect("delete failed"));
        assert!(get(&pool, &row.id).await.expect("get failed").is_none());
        // Second delete is a no-op.
        assert!(!delete(&pool, &row.id).await.expect("second delete failed"));
    }

    /// exists() returns true for a known id, false for an unknown one.
    #[tokio::test]
    async fn exists_true_false() {
        let pool = connect_in_memory().await;

        assert!(!exists(&pool, "no-such-id").await.expect("exists failed"));

        let row = create(&pool, &minimal_input("Vega", "cli", "own"))
            .await
            .expect("create failed");
        assert!(exists(&pool, &row.id).await.expect("exists failed"));
        assert!(!exists(&pool, "wrong-id").await.expect("exists failed"));
    }

    /// list_with_counts returns in_workspaces = 0 for a freshly-created definition.
    #[tokio::test]
    async fn list_with_counts_zero_for_fresh_def() {
        let pool = connect_in_memory().await;

        create(&pool, &minimal_input("Echo", "chat", "own"))
            .await
            .expect("create failed");

        let items = list_with_counts(&pool)
            .await
            .expect("list_with_counts failed");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Echo");
        assert_eq!(items[0].in_workspaces, 0);
    }

    /// JSON must use camelCase keys — locks the TS AgentDefinition contract.
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;

        let row = create(
            &pool,
            &AgentDefinitionInput {
                name: "Sol".into(),
                role: None,
                role_id: None,
                agent_type: "cli".into(),
                cli_kind: Some("claude-code".into()),
                color: None,
                default_level: Some("mid".into()),
                provider_id: None,
                model: None,
                effort: None,
                harness_mode: "own".into(),
                share_blackboard: Some(true),
                auto_submit_injected: Some(true),
                allowed_senders: Some("all".into()),
                permission_mode: Some("auto".into()),
                custom_args: Some("--foo".into()),
                custom_env: Some(r#"{"ANTHROPIC_BASE_URL":"https://x"}"#.into()),
                secret_env_keys: Some(r#"["ANTHROPIC_AUTH_TOKEN"]"#.into()),
                context_window: Some("1m".into()),
                selected_builtin_skill_ids: None,
                rtk_enabled: Some(false),
            },
        )
        .await
        .expect("create failed");

        let json = serde_json::to_value(&row).expect("serialize failed");

        // JSON-text columns serialize to STRUCTURED JSON, not opaque strings.
        assert!(
            json.get("customEnv")
                .and_then(|v| v.get("ANTHROPIC_BASE_URL"))
                .is_some(),
            "customEnv must serialize as an object"
        );
        assert!(
            json.get("secretEnvKeys")
                .and_then(|v| v.as_array())
                .is_some(),
            "secretEnvKeys must serialize as an array"
        );
        assert!(
            json.get("permissionMode").is_some(),
            "must have permissionMode"
        );
        assert!(
            json.get("contextWindow").is_some(),
            "must have contextWindow"
        );
        assert!(json.get("custom_env").is_none(), "must NOT have custom_env");
        assert!(
            json.get("secret_env_keys").is_none(),
            "must NOT have secret_env_keys"
        );

        // camelCase keys present
        assert!(json.get("harnessMode").is_some(), "must have harnessMode");
        assert!(json.get("createdAt").is_some(), "must have createdAt");
        assert!(json.get("cliKind").is_some(), "must have cliKind");
        assert!(
            json.get("autoSubmitInjected").is_some(),
            "must have autoSubmitInjected"
        );
        assert!(
            json.get("shareBlackboard").is_some(),
            "must have shareBlackboard"
        );
        assert_eq!(
            json.get("defaultLevel").and_then(|v| v.as_str()),
            Some("mid")
        );
        assert!(
            json.get("allowedSenders").is_some(),
            "must have allowedSenders"
        );
        assert_eq!(
            json.get("rtkEnabled"),
            Some(&serde_json::Value::Bool(false)),
            "must have rtkEnabled as a JSON bool"
        );

        // snake_case must NOT appear
        assert!(
            json.get("harness_mode").is_none(),
            "must NOT have harness_mode"
        );
        assert!(json.get("created_at").is_none(), "must NOT have created_at");
        assert!(json.get("cli_kind").is_none(), "must NOT have cli_kind");
        assert!(
            json.get("auto_submit_injected").is_none(),
            "must NOT have auto_submit_injected"
        );
        assert!(
            json.get("share_blackboard").is_none(),
            "must NOT have share_blackboard"
        );
        assert!(
            json.get("default_level").is_none(),
            "must NOT have default_level"
        );
        assert!(
            json.get("allowed_senders").is_none(),
            "must NOT have allowed_senders"
        );
        assert!(
            json.get("rtk_enabled").is_none(),
            "must NOT have rtk_enabled"
        );

        // list item also serializes inWorkspaces in camelCase
        let items = list_with_counts(&pool).await.expect("list failed");
        let item_json = serde_json::to_value(&items[0]).expect("serialize failed");
        assert!(
            item_json.get("inWorkspaces").is_some(),
            "must have inWorkspaces"
        );
        assert!(
            item_json.get("in_workspaces").is_none(),
            "must NOT have in_workspaces"
        );
        assert_eq!(
            item_json.get("rtkEnabled"),
            Some(&serde_json::Value::Bool(false)),
            "list_with_counts hardcoded column list must include rtk_enabled"
        );
    }

    #[tokio::test]
    async fn create_and_update_roundtrip_selected_builtin_skill_ids() {
        let pool = connect_in_memory().await;
        let input = AgentDefinitionInput {
            name: "A".into(),
            agent_type: "cli".into(),
            harness_mode: "own".into(),
            selected_builtin_skill_ids: Some(serde_json::json!(["fix-optional"]).to_string()),
            ..Default::default()
        };
        let row = super::create(&pool, &input).await.expect("create failed");
        assert_eq!(
            row.selected_builtin_skill_ids.as_deref(),
            Some(r#"["fix-optional"]"#)
        );

        let fetched = super::get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(
            fetched.selected_builtin_skill_ids,
            row.selected_builtin_skill_ids
        );

        let cleared_input = AgentDefinitionInput {
            selected_builtin_skill_ids: None,
            ..input
        };
        let updated = super::update(&pool, &row.id, &cleared_input)
            .await
            .expect("update failed")
            .expect("row should exist after update");
        assert!(
            updated.selected_builtin_skill_ids.is_none(),
            "update with None must clear the column"
        );
    }

    #[tokio::test]
    async fn numeric_context_window_roundtrips_as_string() {
        let pool = connect_in_memory().await;
        let input = AgentDefinitionInput {
            name: "Codex".into(),
            agent_type: "cli".into(),
            cli_kind: Some("codex".into()),
            model: Some("gpt-5.3-codex-spark".into()),
            harness_mode: "own".into(),
            context_window: Some("121600".into()),
            ..Default::default()
        };

        let row = create(&pool, &input).await.expect("create failed");
        assert_eq!(row.context_window.as_deref(), Some("121600"));

        let fetched = get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched.context_window.as_deref(), Some("121600"));

        let listed = list_with_counts(&pool).await.expect("list failed");
        assert_eq!(listed[0].context_window.as_deref(), Some("121600"));
    }

    /// `rtk_enabled` tri-state round-trip (Task A4 wire contract): unset stays
    /// `None` (== enabled), an explicit `Some(false)` persists as disabled, and
    /// `update` can flip it back to `Some(true)` (explicit enable).
    #[tokio::test]
    async fn rtk_enabled_tristate_roundtrip() {
        let pool = connect_in_memory().await;

        // Save without rtk_enabled -> None (means enabled by default).
        let unset = create(&pool, &minimal_input("Unset", "cli", "own"))
            .await
            .expect("create failed");
        assert!(unset.rtk_enabled.is_none());
        let fetched_unset = get(&pool, &unset.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert!(fetched_unset.rtk_enabled.is_none());

        // Save with rtk_enabled: Some(false) -> disabled, persists as such.
        let disabled_input = AgentDefinitionInput {
            name: "Disabled".into(),
            agent_type: "cli".into(),
            harness_mode: "own".into(),
            rtk_enabled: Some(false),
            ..Default::default()
        };
        let disabled = create(&pool, &disabled_input).await.expect("create failed");
        assert_eq!(disabled.rtk_enabled, Some(false));
        let fetched_disabled = get(&pool, &disabled.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched_disabled.rtk_enabled, Some(false));

        // update() can flip it back to explicit Some(true).
        let re_enabled_input = AgentDefinitionInput {
            rtk_enabled: Some(true),
            ..disabled_input
        };
        let re_enabled = update(&pool, &disabled.id, &re_enabled_input)
            .await
            .expect("update failed")
            .expect("row should exist after update");
        assert_eq!(re_enabled.rtk_enabled, Some(true));
    }

    #[tokio::test]
    async fn antigravity_effort_roundtrips_and_other_harnesses_normalize_to_null() {
        let pool = connect_in_memory().await;
        let input = AgentDefinitionInput {
            name: "AGY".into(),
            agent_type: "cli".into(),
            cli_kind: Some("antigravity".into()),
            model: Some("gemini-pro".into()),
            effort: Some("medium".into()),
            harness_mode: "own".into(),
            ..Default::default()
        };

        let row = create(&pool, &input).await.unwrap();
        assert_eq!(row.effort.as_deref(), Some("medium"));
        assert_eq!(
            get(&pool, &row.id).await.unwrap().unwrap().effort,
            row.effort
        );
        assert_eq!(
            list(&pool).await.unwrap()[0].effort.as_deref(),
            Some("medium")
        );
        assert_eq!(
            list_with_counts(&pool).await.unwrap()[0].effort.as_deref(),
            Some("medium")
        );

        let updated = update(
            &pool,
            &row.id,
            &AgentDefinitionInput {
                effort: Some("high".into()),
                ..input.clone()
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(updated.effort.as_deref(), Some("high"));

        let normalized = update(
            &pool,
            &row.id,
            &AgentDefinitionInput {
                cli_kind: Some("codex".into()),
                effort: Some("low".into()),
                ..input
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert!(normalized.effort.is_none());
    }

    #[tokio::test]
    async fn antigravity_effort_check_rejects_unknown_value() {
        let pool = connect_in_memory().await;
        let error = create(
            &pool,
            &AgentDefinitionInput {
                name: "AGY".into(),
                agent_type: "cli".into(),
                cli_kind: Some("antigravity".into()),
                effort: Some("extreme".into()),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect_err("schema CHECK must reject invalid effort");
        assert!(error.to_string().contains("CHECK constraint failed"));
    }
}

//! Blackboard command handlers (M3.3) — the per-workspace shared key/value store
//! with an activity log and access-scope enforcement.
//!
//! # value encoding
//!
//! `BlackboardEntry.value` is arbitrary JSON (`unknown` in TS). On `set` we
//! `serde_json::to_string` it into the TEXT `value` column (a JSON string). On
//! read we parse that text back into a real `serde_json::Value` for the wire
//! shape ([`entry_to_json`]) — so the UI receives real JSON, not a string-of-JSON.
//! A `None`/absent value omits the `value` key.
//!
//! # access scope
//!
//! A writer/reader instance must belong to the workspace it touches: when a
//! `writerId`/`readerId` is supplied, we fetch the `workspace_agent` and require
//! its `workspace_id == workspaceId`, else [`AppError::Invalid`]. A missing
//! instance id → [`AppError::NotFound`]; a missing workspace → [`AppError::NotFound`].

use crate::engine::repo::blackboard::BlackboardEntryRow;
use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::{json, Value};

/// Default number of activity rows returned alongside `blackboard.list`.
const DEFAULT_ACTIVITY_LIMIT: i64 = 50;

/// Build the camelCase `BlackboardEntry` wire shape from a DB row, parsing the
/// stored `value` text back into a real JSON value (omitted when `None`).
///
/// Parse-failure fallback: treat the raw text as a JSON string value. We always
/// store valid JSON, so this shouldn't trigger — but it must not panic.
fn entry_to_json(row: &BlackboardEntryRow) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), json!(row.id));
    obj.insert("workspaceId".into(), json!(row.workspace_id));
    obj.insert("key".into(), json!(row.key));
    if let Some(text) = &row.value {
        let parsed =
            serde_json::from_str::<Value>(text).unwrap_or_else(|_| Value::String(text.clone()));
        obj.insert("value".into(), parsed);
    }
    if let Some(writer) = &row.last_writer_id {
        obj.insert("lastWriterId".into(), json!(writer));
    }
    obj.insert("updatedAt".into(), json!(row.updated_at));
    Value::Object(obj)
}

/// Enforce that `instance_id` exists AND belongs to `workspace_id`.
///
/// - unknown instance → [`AppError::NotFound`]
/// - instance in a different workspace → [`AppError::Invalid`] (access-scope
///   violation)
async fn enforce_scope(
    state: &AppState,
    workspace_id: &str,
    instance_id: &str,
    role: &str,
) -> Result<(), AppError> {
    let agent = repo::workspace_agent::get(&state.db, instance_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("instance id={instance_id} not found")))?;
    if agent.workspace_id != workspace_id {
        return Err(AppError::Invalid(format!(
            "{role} does not belong to this workspace"
        )));
    }
    Ok(())
}

/// Validate that `workspace_id` exists, else [`AppError::NotFound`].
async fn require_workspace(state: &AppState, workspace_id: &str) -> Result<(), AppError> {
    if !repo::workspace::exists(&state.db, workspace_id).await? {
        return Err(AppError::NotFound(format!(
            "workspace id={workspace_id} not found"
        )));
    }
    Ok(())
}

/// Payload for `blackboard.set`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetReq {
    workspace_id: String,
    key: String,
    value: Value,
    writer_id: Option<String>,
}

/// Set (UPSERT) a blackboard key/value for a workspace.
///
/// `value` is arbitrary JSON, stored as a JSON string. When `writerId` is
/// supplied it must belong to the workspace (access scope), and the write is
/// logged to the activity log. Returns the `BlackboardEntry` wire shape.
///
/// Errors:
/// - malformed payload → [`AppError::Invalid`]
/// - unknown workspace → [`AppError::NotFound`]
/// - unknown writer instance → [`AppError::NotFound`]
/// - writer in a different workspace → [`AppError::Invalid`]
pub async fn set(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let SetReq {
        workspace_id,
        key,
        value,
        writer_id,
    } = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    require_workspace(state, &workspace_id).await?;
    if let Some(writer) = &writer_id {
        enforce_scope(state, &workspace_id, writer, "writer").await?;
    }

    let value_json =
        serde_json::to_string(&value).map_err(|e| AppError::Internal(e.to_string()))?;
    let row = repo::blackboard::set(
        &state.db,
        &workspace_id,
        &key,
        &value_json,
        writer_id.as_deref(),
    )
    .await?;

    Ok(entry_to_json(&row))
}

/// Payload for `blackboard.get`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetReq {
    workspace_id: String,
    key: String,
    reader_id: Option<String>,
}

/// Get a blackboard value by key. When `readerId` is supplied it must belong to
/// the workspace (access scope), and a found read is logged. Returns the
/// `BlackboardEntry` wire shape, or JSON `null` when the key is absent.
///
/// Errors mirror [`set`] (with a reader instead of a writer).
pub async fn get(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let GetReq {
        workspace_id,
        key,
        reader_id,
    } = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    require_workspace(state, &workspace_id).await?;
    if let Some(reader) = &reader_id {
        enforce_scope(state, &workspace_id, reader, "reader").await?;
    }

    let row = repo::blackboard::get(&state.db, &workspace_id, &key, reader_id.as_deref()).await?;
    Ok(match row {
        Some(r) => entry_to_json(&r),
        None => Value::Null,
    })
}

/// Payload for `blackboard.list`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListReq {
    workspace_id: String,
}

/// List all entries for a workspace (newest-first) plus its recent activity
/// (the newest [`DEFAULT_ACTIVITY_LIMIT`] = 50 rows; a busier workspace's older
/// activity is truncated — pagination is a later concern).
///
/// Returns `{ entries: BlackboardEntry[], activity: BlackboardActivity[] }`.
///
/// Errors:
/// - malformed payload → [`AppError::Invalid`]
/// - unknown workspace → [`AppError::NotFound`]
pub async fn list(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let ListReq { workspace_id } =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    require_workspace(state, &workspace_id).await?;

    let entries = repo::blackboard::list(&state.db, &workspace_id).await?;
    let entries: Vec<Value> = entries.iter().map(entry_to_json).collect();

    let activity =
        repo::blackboard::list_activity(&state.db, &workspace_id, DEFAULT_ACTIVITY_LIMIT).await?;
    let activity = serde_json::to_value(activity).map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(json!({ "entries": entries, "activity": activity }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        workspace, workspace_agent,
    };
    use serde_json::json;

    /// Create a workspace, return its id.
    async fn fixture_workspace(state: &AppState) -> String {
        workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed")
            .id
    }

    /// Instantiate an instance in `workspace_id`, return its id.
    async fn fixture_instance(state: &AppState, workspace_id: &str, name: &str) -> String {
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: name.into(),
                role: None,
                agent_type: "cli".into(),
                cli_kind: None,
                color: None,
                provider_id: None,
                model: None,
                harness_mode: "own".into(),
                share_blackboard: None,
                auto_submit_injected: None,
                allowed_senders: None,
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::instantiate(&state.db, workspace_id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    /// set → get round-trips a structured value as parsed JSON (not a string).
    #[tokio::test]
    async fn set_get_roundtrip_structured_value() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;

        set(
            &state,
            json!({ "workspaceId": ws, "key": "cfg", "value": { "n": 1, "xs": [true, "y"] } }),
        )
        .await
        .expect("set failed");

        let got = get(&state, json!({ "workspaceId": ws, "key": "cfg" }))
            .await
            .expect("get failed");

        // value comes back as parsed JSON, not a string-of-JSON.
        let v = got.get("value").expect("value present");
        assert!(v.is_object(), "value must be parsed JSON object, got {v:?}");
        assert_eq!(v.get("n").and_then(Value::as_i64), Some(1));
        assert_eq!(
            got.get("workspaceId").and_then(Value::as_str),
            Some(ws.as_str())
        );
    }

    /// set is an UPSERT: writing the same key twice updates and keeps one entry.
    #[tokio::test]
    async fn set_upserts_single_entry() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let agent = fixture_instance(&state, &ws, "Writer").await;

        let r1 = set(&state, json!({ "workspaceId": ws, "key": "k", "value": 1 }))
            .await
            .expect("first set");
        let r2 = set(
            &state,
            json!({ "workspaceId": ws, "key": "k", "value": 2, "writerId": agent }),
        )
        .await
        .expect("second set");

        assert_eq!(
            r1.get("id"),
            r2.get("id"),
            "UPSERT must keep the same entry id"
        );
        assert_eq!(r2.get("value").and_then(Value::as_i64), Some(2));
        assert_eq!(
            r2.get("lastWriterId").and_then(Value::as_str),
            Some(agent.as_str())
        );

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list");
        let entries = listed.get("entries").and_then(Value::as_array).unwrap();
        assert_eq!(entries.len(), 1, "UPSERT must not create a second entry");
    }

    /// Agent write logs activity; user write logs none.
    #[tokio::test]
    async fn activity_logged_for_agent_write_only() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let agent = fixture_instance(&state, &ws, "Writer").await;

        // User write — no activity.
        set(&state, json!({ "workspaceId": ws, "key": "u", "value": 1 }))
            .await
            .expect("user set");
        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list");
        assert!(listed
            .get("activity")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty());

        // Agent write — one 'write'.
        set(
            &state,
            json!({ "workspaceId": ws, "key": "a", "value": 1, "writerId": agent }),
        )
        .await
        .expect("agent set");
        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list");
        let acts = listed.get("activity").and_then(Value::as_array).unwrap();
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].get("action").and_then(Value::as_str), Some("write"));
        assert_eq!(
            acts[0].get("instanceId").and_then(Value::as_str),
            Some(agent.as_str())
        );
    }

    /// get with readerId logs a 'read'; missing key → null.
    #[tokio::test]
    async fn get_reader_activity_and_missing_null() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let reader = fixture_instance(&state, &ws, "Reader").await;
        set(&state, json!({ "workspaceId": ws, "key": "k", "value": 1 }))
            .await
            .expect("set");

        get(
            &state,
            json!({ "workspaceId": ws, "key": "k", "readerId": reader }),
        )
        .await
        .expect("get");
        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list");
        let acts = listed.get("activity").and_then(Value::as_array).unwrap();
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].get("action").and_then(Value::as_str), Some("read"));

        // Missing key WITH a reader → JSON null, and NO activity is logged
        // (there's no entry to attribute the read to).
        let missing = get(
            &state,
            json!({ "workspaceId": ws, "key": "nope", "readerId": reader }),
        )
        .await
        .expect("get missing");
        assert!(missing.is_null());
        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list");
        let acts = listed.get("activity").and_then(Value::as_array).unwrap();
        assert_eq!(acts.len(), 1, "missing-key read must not add activity");
    }

    /// Access scope: a writer from another workspace → Invalid.
    #[tokio::test]
    async fn set_cross_workspace_writer_invalid() {
        let state = AppState::for_tests().await;
        let ws1 = fixture_workspace(&state).await;
        let ws2 = fixture_workspace(&state).await;
        let foreigner = fixture_instance(&state, &ws2, "Foreign").await;

        let err = set(
            &state,
            json!({ "workspaceId": ws1, "key": "k", "value": 1, "writerId": foreigner }),
        )
        .await
        .expect_err("cross-workspace writer must fail");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    /// Access scope: a reader from another workspace → Invalid.
    #[tokio::test]
    async fn get_cross_workspace_reader_invalid() {
        let state = AppState::for_tests().await;
        let ws1 = fixture_workspace(&state).await;
        let ws2 = fixture_workspace(&state).await;
        let foreigner = fixture_instance(&state, &ws2, "Foreign").await;
        set(
            &state,
            json!({ "workspaceId": ws1, "key": "k", "value": 1 }),
        )
        .await
        .expect("set");

        let err = get(
            &state,
            json!({ "workspaceId": ws1, "key": "k", "readerId": foreigner }),
        )
        .await
        .expect_err("cross-workspace reader must fail");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    /// Unknown workspace → NotFound.
    #[tokio::test]
    async fn unknown_workspace_not_found() {
        let state = AppState::for_tests().await;
        let err = set(
            &state,
            json!({ "workspaceId": "nope", "key": "k", "value": 1 }),
        )
        .await
        .expect_err("unknown workspace must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// Unknown instance id → NotFound.
    #[tokio::test]
    async fn unknown_instance_not_found() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let err = set(
            &state,
            json!({ "workspaceId": ws, "key": "k", "value": 1, "writerId": "nope" }),
        )
        .await
        .expect_err("unknown writer must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// list returns the workspace's entries (newest-first); other workspaces
    /// excluded.
    #[tokio::test]
    async fn list_scoped_newest_first() {
        let state = AppState::for_tests().await;
        let ws1 = fixture_workspace(&state).await;
        let ws2 = fixture_workspace(&state).await;

        set(
            &state,
            json!({ "workspaceId": ws1, "key": "a", "value": 1 }),
        )
        .await
        .expect("set a");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        set(
            &state,
            json!({ "workspaceId": ws1, "key": "b", "value": 2 }),
        )
        .await
        .expect("set b");
        set(
            &state,
            json!({ "workspaceId": ws2, "key": "c", "value": 3 }),
        )
        .await
        .expect("set c");

        let listed = list(&state, json!({ "workspaceId": ws1 }))
            .await
            .expect("list");
        let entries = listed.get("entries").and_then(Value::as_array).unwrap();
        assert_eq!(entries.len(), 2, "only ws1 entries");
        assert_eq!(entries[0].get("key").and_then(Value::as_str), Some("b"));
        assert_eq!(entries[1].get("key").and_then(Value::as_str), Some("a"));
    }
}

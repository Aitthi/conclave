//! `orient.list` — one bounded fresh-context orientation packet (task
//! conclave-orient).
//!
//! A fresh context used to run 3–4 commands to orient (task list, agent
//! list, msg list, bb list), together tens of KB. This verb COMPOSES the
//! existing handlers/repo reads into one packet with a hard cap on every
//! section, so `conclave orient <ws>` is the documented first command after
//! any restore/clear.
//!
//! Sections (each capped, see the `*_CAP` consts):
//! `tasks` (the slim `task.list` shape, live states only), `roster`
//! (projected), `messages` (the caller's inbox+outbox, text truncated),
//! `blackboard` (key + value head), `watches` (slugs the caller watches),
//! and `self` (id/name/level plus `contextPct` when known).

use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::{json, Value};

/// Slim task rows included (live states only — the slim list already hides
/// merged/abandoned).
const TASKS_CAP: usize = 30;
/// Roster rows included.
const ROSTER_CAP: usize = 20;
/// Latest inbox+outbox messages included.
const MESSAGES_CAP: usize = 10;
/// Characters of each message's text kept (then an `…` marker).
const MESSAGE_TEXT_CAP: usize = 200;
/// Newest blackboard entries included.
const BLACKBOARD_CAP: usize = 20;
/// Characters of each blackboard value kept (then an `…` marker).
const BLACKBOARD_VALUE_CAP: usize = 80;

/// Validate that `workspace_id` exists, else [`AppError::NotFound`] (same
/// local helper convention as every other command module).
async fn require_workspace(state: &AppState, workspace_id: &str) -> Result<(), AppError> {
    if !repo::workspace::exists(&state.db, workspace_id).await? {
        return Err(AppError::NotFound(format!(
            "workspace id={workspace_id} not found"
        )));
    }
    Ok(())
}

/// Enforce that `instance_id` exists AND belongs to `workspace_id` (mirrors
/// `commands::task::enforce_scope`) — orienting as a foreign/typo'd id fails
/// loudly instead of returning someone else's inbox.
async fn enforce_scope(
    state: &AppState,
    workspace_id: &str,
    instance_id: &str,
) -> Result<(), AppError> {
    let agent = repo::workspace_agent::get(&state.db, instance_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("actor id={instance_id} not found")))?;
    if agent.workspace_id != workspace_id {
        return Err(AppError::Invalid(
            "actor does not belong to this workspace".into(),
        ));
    }
    Ok(())
}

/// First `max_chars` CHARS of `text` with an `…` marker when truncated —
/// char-boundary-safe (message/blackboard text is frequently non-ASCII).
fn truncate_chars(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => format!("{}…", &text[..idx]),
        None => text.to_string(),
    }
}

/// A blackboard value as a one-line preview string: strings verbatim, any
/// other JSON compact-serialized, then char-capped.
fn value_preview(value: Option<&Value>, max_chars: usize) -> String {
    let text = match value {
        None => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    };
    truncate_chars(&text, max_chars)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrientReq {
    workspace_id: String,
    actor_id: String,
}

/// Build the orientation packet. Every section REUSES an existing handler or
/// repo read — this module owns only the composition, projection, and caps.
pub async fn list(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: OrientReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    enforce_scope(state, &req.workspace_id, &req.actor_id).await?;

    // 1. tasks — the slim `task.list` shape (cli-output-economy), capped.
    let slim = super::task::list(
        state,
        json!({ "workspaceId": req.workspace_id, "slim": true }),
    )
    .await?;
    let slim_rows = match slim {
        Value::Array(rows) => rows,
        _ => Vec::new(),
    };
    let tasks_total = slim_rows.len();
    let tasks: Vec<Value> = slim_rows.into_iter().take(TASKS_CAP).collect();

    // 2. roster — projected from `instance.list` (name/id/level/working/model
    // only; `agent list` stays the deep read).
    let roster_full =
        super::instance::list(state, json!({ "workspaceId": req.workspace_id })).await?;
    let roster_rows = roster_full.as_array().cloned().unwrap_or_default();
    let roster: Vec<Value> = roster_rows
        .iter()
        .take(ROSTER_CAP)
        .map(|row| {
            json!({
                "id": row.get("id").cloned().unwrap_or(Value::Null),
                "name": row.get("name").cloned().unwrap_or(Value::Null),
                "level": row.get("level").cloned().unwrap_or(Value::Null),
                "working": row.get("working").cloned().unwrap_or(Value::Null),
                "model": row.get("model").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    // 3. messages — the caller's inbox+outbox via `message.list`'s data path,
    // latest MESSAGES_CAP, text char-capped. Names resolved once server-side
    // (withNames), falling back to the raw instance id.
    let msgs = super::message::list(
        state,
        json!({
            "instanceId": req.actor_id,
            "limit": MESSAGES_CAP,
            "withNames": true,
        }),
    )
    .await?;
    let messages: Vec<Value> = msgs
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|row| {
            let party = |name_key: &str, id_key: &str| -> Value {
                row.get(name_key)
                    .or_else(|| row.get(id_key))
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            let text = row.get("text").and_then(Value::as_str).unwrap_or("");
            json!({
                "from": party("fromName", "fromInstanceId"),
                "to": party("toName", "toInstanceId"),
                "text": truncate_chars(text, MESSAGE_TEXT_CAP),
                "createdAt": row.get("createdAt").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    // 4. blackboard — key + value head, newest first, capped.
    let bb = super::blackboard::list(state, json!({ "workspaceId": req.workspace_id })).await?;
    let blackboard: Vec<Value> = bb
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .take(BLACKBOARD_CAP)
        .map(|entry| {
            json!({
                "key": entry.get("key").cloned().unwrap_or(Value::Null),
                "value": value_preview(entry.get("value"), BLACKBOARD_VALUE_CAP),
            })
        })
        .collect();

    // 5. watches — slugs of LIVE tasks the caller subscribed to. Reuses the
    // repo reads (task list is already bounded by the workspace's live tasks;
    // one `watchers` lookup each).
    let live_rows = repo::task::list(&state.db, &req.workspace_id, None, false).await?;
    let mut watches: Vec<String> = Vec::new();
    for row in live_rows
        .iter()
        .filter(|row| !matches!(row.state.as_str(), "merged" | "abandoned"))
    {
        if repo::task::watchers(&state.db, &row.id)
            .await?
            .iter()
            .any(|id| id == &req.actor_id)
        {
            watches.push(row.slug.clone());
        }
    }

    // 6. self — the caller's own roster row, plus contextPct when the meter
    // resolved (contextTokens/contextLimit come from the enriched row).
    let self_row = roster_rows
        .iter()
        .find(|row| row.get("id").and_then(Value::as_str) == Some(req.actor_id.as_str()));
    let self_section = match self_row {
        Some(row) => {
            let mut obj = json!({
                "id": row.get("id").cloned().unwrap_or(Value::Null),
                "name": row.get("name").cloned().unwrap_or(Value::Null),
                "level": row.get("level").cloned().unwrap_or(Value::Null),
            });
            let tokens = row.get("contextTokens").and_then(Value::as_i64);
            let limit = row.get("contextLimit").and_then(Value::as_i64);
            if let (Some(tokens), Some(limit)) = (tokens, limit) {
                if limit > 0 {
                    obj["contextPct"] = json!((100 * tokens.max(0) / limit).min(100));
                }
            }
            obj
        }
        None => Value::Null,
    };

    Ok(json!({
        "tasks": tasks,
        "tasksTotal": tasks_total,
        "roster": roster,
        "messages": messages,
        "blackboard": blackboard,
        "watches": watches,
        "self": self_section,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        workspace, workspace_agent,
    };

    async fn fixture_workspace(state: &AppState) -> String {
        workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed")
            .id
    }

    async fn fixture_instance(state: &AppState, workspace_id: &str, name: &str) -> String {
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: name.into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
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

    #[test]
    fn truncate_chars_is_char_boundary_safe() {
        assert_eq!(truncate_chars("short", 10), "short");
        assert_eq!(truncate_chars("exact", 5), "exact");
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
        // Thai: 3-byte chars — a byte-index cut would panic.
        assert_eq!(truncate_chars("สวัสดีครับ", 4), "สวัส…");
    }

    #[test]
    fn value_preview_strings_verbatim_and_json_compact() {
        assert_eq!(value_preview(Some(&json!("plain")), 80), "plain");
        assert_eq!(
            value_preview(Some(&json!({"a": 1})), 80),
            "{\"a\":1}".to_string()
        );
        assert_eq!(value_preview(None, 80), "");
        assert_eq!(value_preview(Some(&json!("abcdef")), 3), "abc…");
    }

    #[tokio::test]
    async fn orient_packet_carries_all_sections_with_caps() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let me = fixture_instance(&state, &ws, "Me").await;
        let peer = fixture_instance(&state, &ws, "Peer").await;

        // One live task (watched by me) and one merged task (must not count).
        super::super::task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t-live", "title": "Live", "plan": "p" }),
        )
        .await
        .expect("create t-live");
        super::super::task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t-done", "title": "Done" }),
        )
        .await
        .expect("create t-done");
        super::super::task::claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t-done", "actorId": peer }),
        )
        .await
        .expect("claim t-done");
        for next in ["in_progress", "review", "merged"] {
            super::super::task::set_state(
                &state,
                json!({ "workspaceId": ws, "slug": "t-done", "state": next, "actorId": peer }),
            )
            .await
            .expect("walk t-done");
        }
        super::super::task::watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t-live", "actorId": me }),
        )
        .await
        .expect("watch t-live");

        // A long inbound message (truncation) and a blackboard key.
        let long_text = "x".repeat(300);
        repo::inter_agent_message::create(&state.db, &peer, &me, &long_text, "delivered", true)
            .await
            .expect("message");
        repo::blackboard::set(&state.db, &ws, "k1", "\"v1\"", Some(&peer))
            .await
            .expect("bb set");

        let packet = list(&state, json!({ "workspaceId": ws, "actorId": me }))
            .await
            .expect("orient failed");

        // tasks: slim shape, live only.
        let tasks = packet["tasks"].as_array().expect("tasks array");
        assert_eq!(tasks.len(), 1, "merged task hidden");
        assert_eq!(tasks[0]["slug"], json!("t-live"));
        assert!(tasks[0].get("plan").is_none(), "slim rows carry no plan");
        assert_eq!(packet["tasksTotal"], json!(1));

        // roster: projected fields only.
        let roster = packet["roster"].as_array().expect("roster array");
        assert_eq!(roster.len(), 2);
        assert!(roster[0].get("skillNames").is_none(), "no skills arrays");

        // messages: latest first, text truncated at 200 chars + marker.
        let messages = packet["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 1);
        let text = messages[0]["text"].as_str().expect("text");
        assert_eq!(text.chars().count(), 201, "200 chars + … marker");
        assert!(text.ends_with('…'));
        assert_eq!(messages[0]["from"], json!("Peer"), "withNames resolved");

        // blackboard: key + value head.
        let bb = packet["blackboard"].as_array().expect("blackboard array");
        assert_eq!(bb.len(), 1);
        assert_eq!(bb[0]["key"], json!("k1"));
        assert_eq!(bb[0]["value"], json!("v1"));

        // watches + self.
        assert_eq!(packet["watches"], json!(["t-live"]));
        assert_eq!(packet["self"]["name"], json!("Me"));
        assert_eq!(packet["self"]["id"], json!(me));
        // contextPct is present only when the meter resolved; when it is,
        // it must be a clamped integer percentage.
        if let Some(pct) = packet["self"].get("contextPct") {
            let pct = pct.as_i64().expect("contextPct is an integer");
            assert!((0..=100).contains(&pct), "contextPct in range, got {pct}");
        }
    }

    #[tokio::test]
    async fn orient_caps_tasks_at_thirty_and_reports_total() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let me = fixture_instance(&state, &ws, "Me").await;
        for i in 0..35 {
            super::super::task::create(
                &state,
                json!({ "workspaceId": ws, "slug": format!("t{i}"), "title": format!("T{i}") }),
            )
            .await
            .expect("create");
        }

        let packet = list(&state, json!({ "workspaceId": ws, "actorId": me }))
            .await
            .expect("orient failed");
        assert_eq!(packet["tasks"].as_array().expect("tasks").len(), TASKS_CAP);
        assert_eq!(packet["tasksTotal"], json!(35));
    }

    #[tokio::test]
    async fn orient_rejects_foreign_actor_and_unknown_workspace() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let other_ws = fixture_workspace(&state).await;
        let outsider = fixture_instance(&state, &other_ws, "Outsider").await;

        let foreign = list(&state, json!({ "workspaceId": ws, "actorId": outsider })).await;
        assert!(matches!(foreign, Err(AppError::Invalid(_))));

        let missing = list(
            &state,
            json!({ "workspaceId": "nope", "actorId": outsider }),
        )
        .await;
        assert!(matches!(missing, Err(AppError::NotFound(_))));
    }
}

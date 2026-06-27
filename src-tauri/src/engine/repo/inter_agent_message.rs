//! InterAgentMessage repository — a line of text injected from one instance into
//! another instance's live input (the inter-agent messaging backbone, M3.1).
//!
//! One `inter_agent_message` row is persisted per `message.inject` call. The
//! `status` reflects whether the text reached a live backend (`delivered`) or
//! was merely recorded because the target wasn't running (`queued`).
//!
//! # chain-builder usage
//!
//! [`create`] uses chain-builder for a single-table INSERT — same idiom as
//! `session::create_for_instance` / `workspace_agent::create`. No raw sqlx is
//! needed. A future read path (M3.2 inbox/outbox queries) will add the SELECT
//! helpers; this module is intentionally limited to `create` + the row.

use super::cb_err;
use chain_builder::{QueryBuilder, Sqlite, Value as Bind};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Row struct ──────────────────────────────────────────────────────────────

/// Decoded row from the `inter_agent_message` table.
///
/// `sqlx::FromRow` maps snake_case column names to snake_case fields.
/// `serde(rename_all = "camelCase")` emits camelCase JSON matching the
/// `InterAgentMessage` interface in `src/ipc/types.ts`.
///
/// SQLite stores booleans as INTEGER; sqlx maps the nullable `auto_submitted`
/// column to `Option<bool>` fine. `skip_serializing_if` keeps the key absent
/// (not `null`) when `None`, matching the optional `autoSubmitted?` TS field.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InterAgentMessageRow {
    pub id: String,
    pub from_instance_id: String, // → "fromInstanceId"
    pub to_instance_id: String,   // → "toInstanceId"
    pub text: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_submitted: Option<bool>, // → "autoSubmitted"
    pub created_at: String, // → "createdAt"
}

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Insert a new inter_agent_message and return the constructed row.
///
/// Generates a UUID v4 `id` and ISO-8601 UTC `created_at` timestamp.
/// `status` must be one of `"queued"` | `"delivered"` (the schema CHECK
/// enforces this). `auto_submitted` is always `true` for an injection.
///
/// Returns the row directly without a re-fetch round-trip (same as the other
/// repo `create` helpers).
pub async fn create(
    pool: &SqlitePool,
    from_instance_id: &str,
    to_instance_id: &str,
    text: &str,
    status: &str,
    auto_submitted: bool,
) -> sqlx::Result<InterAgentMessageRow> {
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let from_instance_id = from_instance_id.to_owned();
    let to_instance_id = to_instance_id.to_owned();
    let text = text.to_owned();
    let status = status.to_owned();

    QueryBuilder::<Sqlite>::table("inter_agent_message")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("from_instance_id", Bind::Text(from_instance_id.clone())),
            ("to_instance_id", Bind::Text(to_instance_id.clone())),
            ("text", Bind::Text(text.clone())),
            ("status", Bind::Text(status.clone())),
            ("auto_submitted", Bind::Bool(auto_submitted)),
            ("created_at", Bind::Text(created_at.clone())),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(InterAgentMessageRow {
        id,
        from_instance_id,
        to_instance_id,
        text,
        status,
        auto_submitted: Some(auto_submitted),
        created_at,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        db::connect_in_memory,
        repo::{
            agent_definition::{self, AgentDefinitionInput},
            workspace, workspace_agent,
        },
    };

    /// Helper: create a workspace + agent_definition, instantiate, and return
    /// the resulting `workspace_agent` id (a valid FK target).
    async fn fixture_instance(pool: &SqlitePool, name: &str) -> String {
        let ws = workspace::create(pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            pool,
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
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::instantiate(pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    /// create returns a fully-populated row with the given fields.
    #[tokio::test]
    async fn create_populates_row() {
        let pool = connect_in_memory().await;
        let from = fixture_instance(&pool, "Sender").await;
        let to = fixture_instance(&pool, "Target").await;

        let row = create(&pool, &from, &to, "hello", "delivered", true)
            .await
            .expect("create failed");

        assert_eq!(row.from_instance_id, from);
        assert_eq!(row.to_instance_id, to);
        assert_eq!(row.text, "hello");
        assert_eq!(row.status, "delivered");
        assert_eq!(row.auto_submitted, Some(true));
        assert!(!row.id.is_empty());
        assert!(!row.created_at.is_empty());
    }

    /// JSON serialization uses camelCase — locks the TS InterAgentMessage contract.
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;
        let from = fixture_instance(&pool, "Sender").await;
        let to = fixture_instance(&pool, "Target").await;

        let row = create(&pool, &from, &to, "hi", "queued", true)
            .await
            .expect("create failed");

        let json = serde_json::to_value(&row).expect("serialize failed");

        // camelCase keys must be present
        assert!(
            json.get("fromInstanceId").is_some(),
            "must have fromInstanceId"
        );
        assert!(json.get("toInstanceId").is_some(), "must have toInstanceId");
        assert!(
            json.get("autoSubmitted").is_some(),
            "must have autoSubmitted"
        );
        assert!(json.get("createdAt").is_some(), "must have createdAt");

        // snake_case must NOT appear
        assert!(
            json.get("from_instance_id").is_none(),
            "must NOT have from_instance_id"
        );
        assert!(
            json.get("to_instance_id").is_none(),
            "must NOT have to_instance_id"
        );
        assert!(
            json.get("auto_submitted").is_none(),
            "must NOT have auto_submitted"
        );
        assert!(json.get("created_at").is_none(), "must NOT have created_at");
    }
}

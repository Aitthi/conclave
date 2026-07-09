//! Workspace artifact repository (plan design-artifact-store, Lane A).
//!
//! An artifact is a significant, self-contained agent output — a document,
//! code file, HTML page, SVG, diagram, or React component — persisted via
//! `conclave artifact add` and surfaced live in the Artifacts view. Migration
//! `0014_artifact_workspace.sql` promoted the table from a chat-message child
//! to this workspace-scoped shape; `message_id` stays nullable so the old
//! chat-parsed rows (`kind = 'html'`) keep round-tripping.
//!
//! # Query convention
//!
//! Single-table SELECTs use chain-builder (mirrors `repo::task::get`). The
//! INSERT uses raw `sqlx` because it binds a fixed column list with a
//! generated id + timestamp — the same documented fallback `repo::task::create`
//! uses.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

/// A stored artifact row. Serialises to camelCase across the Tauri IPC
/// boundary. `message_id`/`sandboxed` survive from the pre-0014 chat-parsed
/// rows; new CLI-created artifacts leave them `NULL`.
///
/// Every optional field carries `skip_serializing_if = "Option::is_none"` so
/// an absent value is OMITTED, not emitted as JSON `null` — matching the
/// optional (non-nullable) fields in `src/ipc/types.ts` `Artifact` (same
/// convention as `InterAgentMessageRow::auto_submitted`). `sandboxed` is
/// `Option<bool>`, not `Option<i64>`: SQLite stores the flag as INTEGER but
/// sqlx decodes 0/1 → bool, and the TS contract declares `sandboxed?: boolean`
/// (same as `ToolRow::is_core`).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRow {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandboxed: Option<bool>,
    pub created_at: String,
}

const ARTIFACT_COLS: [&str; 10] = [
    "id",
    "workspace_id",
    "agent_id",
    "message_id",
    "title",
    "kind",
    "filename",
    "content",
    "sandboxed",
    "created_at",
];

/// Insert a workspace-scoped artifact and return the constructed row.
/// `filename` is optional (inline `--content` has no file); `message_id` and
/// `sandboxed` are always `NULL` for CLI-created artifacts.
#[allow(clippy::too_many_arguments)]
pub async fn insert_artifact(
    pool: &SqlitePool,
    workspace_id: &str,
    agent_id: Option<&str>,
    title: &str,
    kind: &str,
    filename: Option<&str>,
    content: &str,
) -> sqlx::Result<ArtifactRow> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO artifact \
         (id, workspace_id, agent_id, message_id, title, kind, filename, content, sandboxed, created_at) \
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, NULL, ?8)",
    )
    .bind(&id)
    .bind(workspace_id)
    .bind(agent_id)
    .bind(title)
    .bind(kind)
    .bind(filename)
    .bind(content)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(ArtifactRow {
        id,
        workspace_id: Some(workspace_id.to_owned()),
        agent_id: agent_id.map(str::to_owned),
        message_id: None,
        title: Some(title.to_owned()),
        kind: Some(kind.to_owned()),
        filename: filename.map(str::to_owned),
        content: Some(content.to_owned()),
        sandboxed: None,
        created_at: now,
    })
}

/// List a workspace's artifacts, newest first (matches the
/// `idx_artifact_ws_created` index). Content is included — the Artifacts view
/// renders each artifact from this one call.
pub async fn list_artifacts(
    pool: &SqlitePool,
    workspace_id: &str,
) -> sqlx::Result<Vec<ArtifactRow>> {
    QueryBuilder::<Sqlite>::table("artifact")
        .select(ARTIFACT_COLS)
        .where_eq("workspace_id", workspace_id)
        .order_by("created_at", Order::Desc)
        .fetch_all::<ArtifactRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Fetch one artifact by id, or `None` if absent.
pub async fn get_artifact(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<ArtifactRow>> {
    QueryBuilder::<Sqlite>::table("artifact")
        .select(ARTIFACT_COLS)
        .where_eq("id", id)
        .fetch_optional::<ArtifactRow, _>(pool)
        .await
        .map_err(cb_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;
    use serde_json::Value;

    async fn fixture_workspace(pool: &SqlitePool) -> String {
        crate::engine::repo::workspace::create(pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed")
            .id
    }

    #[tokio::test]
    async fn insert_then_get_roundtrip() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;

        let art = insert_artifact(
            &pool,
            &ws,
            Some("agent-1"),
            "My Doc",
            "markdown",
            Some("doc.md"),
            "# Hello",
        )
        .await
        .expect("insert failed");

        assert_eq!(art.workspace_id.as_deref(), Some(ws.as_str()));
        assert_eq!(art.kind.as_deref(), Some("markdown"));
        assert!(art.message_id.is_none());

        let fetched = get_artifact(&pool, &art.id)
            .await
            .expect("get failed")
            .expect("artifact exists");
        assert_eq!(fetched, art);
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let pool = connect_in_memory().await;
        assert!(get_artifact(&pool, "nope")
            .await
            .expect("get failed")
            .is_none());
    }

    #[tokio::test]
    async fn list_is_newest_first_and_workspace_scoped() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let other = crate::engine::repo::workspace::create(&pool, "Other", "/tmp/other", None)
            .await
            .expect("create other ws failed")
            .id;

        // created_at is an rfc3339 string; sleep 1ms so ordering is deterministic.
        let a = insert_artifact(&pool, &ws, None, "first", "text", None, "1")
            .await
            .expect("insert a");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let b = insert_artifact(&pool, &ws, None, "second", "text", None, "2")
            .await
            .expect("insert b");
        // A different workspace's artifact must not leak into the ws listing.
        insert_artifact(&pool, &other, None, "elsewhere", "text", None, "x")
            .await
            .expect("insert other");

        let listed = list_artifacts(&pool, &ws).await.expect("list failed");
        assert_eq!(listed.len(), 2, "only this workspace's artifacts");
        assert_eq!(listed[0].id, b.id, "newest first");
        assert_eq!(listed[1].id, a.id);
    }

    #[tokio::test]
    async fn serializes_camel_case() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let art = insert_artifact(&pool, &ws, Some("ag"), "T", "code", None, "x")
            .await
            .expect("insert failed");

        // A new artifact has workspaceId + agentId, but message_id/sandboxed
        // are None → those keys must be OMITTED (not `null`), matching the
        // optional TS fields.
        let json = serde_json::to_value(&art).expect("serialize failed");
        assert_eq!(
            json.get("workspaceId").and_then(Value::as_str),
            Some(ws.as_str())
        );
        assert!(json.get("agentId").is_some(), "must have agentId key");
        assert!(json.get("createdAt").is_some(), "must have createdAt key");
        assert!(
            json.get("workspace_id").is_none(),
            "no snake_case workspace_id"
        );
        assert!(json.get("created_at").is_none(), "no snake_case created_at");
        // skip_serializing_if: absent optionals are omitted, never JSON null.
        assert!(
            json.get("messageId").is_none(),
            "absent messageId must be omitted, not null"
        );
        assert!(
            json.get("sandboxed").is_none(),
            "absent sandboxed must be omitted, not null"
        );
        assert!(
            !json.as_object().unwrap().values().any(Value::is_null),
            "no field serializes as null"
        );
    }

    /// Wire-shape contract for a chat-parsed (`kind='html'`) row: `sandboxed`
    /// must serialise as a JSON BOOLEAN (not the integer 0/1 SQLite stores),
    /// and the null workspace/agent columns must be OMITTED, not `null`. (This
    /// asserts the SERIALISATION shape on a directly-inserted row; the actual
    /// 0014 data-preservation upgrade path — including the serialised shape of a
    /// real message-owned migrated row — is proven by
    /// `db::tests::migrate_0014_preserves_legacy_artifact_rows`.)
    #[tokio::test]
    async fn legacy_html_row_serialises_sandboxed_as_bool() {
        let pool = connect_in_memory().await;
        // message_id NULL keeps this a self-contained serialization unit test
        // (no FK chain needed); the message-owned case lives in the db test.
        sqlx::query(
            "INSERT INTO artifact (id, workspace_id, agent_id, message_id, title, kind, filename, content, sandboxed, created_at) \
             VALUES ('legacy-1', NULL, NULL, NULL, NULL, 'html', 'old.html', '<h1>hi</h1>', 0, '2020-01-01T00:00:00+00:00')",
        )
        .execute(&pool)
        .await
        .expect("insert legacy row");

        let row = get_artifact(&pool, "legacy-1")
            .await
            .expect("get failed")
            .expect("legacy row exists");
        assert_eq!(row.kind.as_deref(), Some("html"));
        assert_eq!(row.content.as_deref(), Some("<h1>hi</h1>"));
        assert_eq!(
            row.sandboxed,
            Some(false),
            "INTEGER 0 decodes to bool false"
        );

        let json = serde_json::to_value(&row).expect("serialize failed");
        assert_eq!(
            json.get("sandboxed"),
            Some(&Value::Bool(false)),
            "sandboxed must be a JSON bool"
        );
        assert!(
            json.get("workspaceId").is_none(),
            "null workspaceId omitted, not null"
        );
        assert!(
            json.get("agentId").is_none(),
            "null agentId omitted, not null"
        );
        assert!(
            json.get("messageId").is_none(),
            "null messageId omitted, not null"
        );
        assert!(
            !json.as_object().unwrap().values().any(Value::is_null),
            "no field is JSON null"
        );
    }
}

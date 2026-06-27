//! Workspace repository — canonical repo pattern for Conclave.
//!
//! # Copying this pattern (M1.4+)
//!
//! To add a new entity (e.g. `agent_definition`), copy this file and:
//!   - Replace `WorkspaceRow` / `"workspace"` with the new entity name/table.
//!   - Adjust columns in `select(...)` to match the table schema.
//!   - Add the new module to `engine/repo/mod.rs`.
//!
//! # chain-builder usage
//!
//! - `list`, `get`, `exists`: chain-builder SELECT with `fetch_all` / `fetch_optional`.
//! - `create`: chain-builder INSERT.  All bind values are cast to
//!   `chain_builder::Value` so the array stays homogeneous even with the
//!   optional `color` column (`None` → `Value::Null`).
//!
//! Chain-builder fetch helpers return `chain_builder::Error`, which is
//! converted to `sqlx::Error` via `cb_err()` so callers get the canonical
//! `sqlx::Result<T>` type and handlers can `?` into `AppError`.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite, Value as Bind};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Row struct ──────────────────────────────────────────────────────────────

/// Decoded row from the `workspace` table.
///
/// `sqlx::FromRow` maps snake_case column names to these snake_case fields.
/// `serde(rename_all = "camelCase")` then emits camelCase JSON that matches
/// the `Workspace` interface in `src/ipc/types.ts`. `skip_serializing_if` on
/// `color` omits the key entirely when `None` (matches the TS `color?: string`
/// optional, not `string | null`).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub folder_path: String, // serializes to "folderPath"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub created_at: String, // serializes to "createdAt"
}

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Return all workspaces ordered by `created_at` ascending, with `id` as a
/// stable tie-breaker (two rows created in the same second would otherwise
/// order non-deterministically).
pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<WorkspaceRow>> {
    QueryBuilder::<Sqlite>::table("workspace")
        .select(["id", "name", "folder_path", "color", "created_at"])
        .order_by("created_at", Order::Asc)
        .order_by("id", Order::Asc)
        .fetch_all::<WorkspaceRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Fetch a single workspace by `id`, or `None` if it does not exist.
pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<WorkspaceRow>> {
    QueryBuilder::<Sqlite>::table("workspace")
        .select(["id", "name", "folder_path", "color", "created_at"])
        .where_eq("id", id)
        .fetch_optional::<WorkspaceRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Return `true` if a workspace with `id` exists.
///
/// Delegates to `get()` — avoids a separate COUNT query and extra trait bounds.
pub async fn exists(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    get(pool, id).await.map(|opt| opt.is_some())
}

/// Insert a new workspace and return the constructed row.
///
/// Generates a UUID v4 `id` and ISO-8601 UTC `created_at` timestamp.
///
/// All INSERT bind values are `chain_builder::Value` so the array is
/// homogeneous: `color: Option<&str>` maps to `Bind::Text(s)` or `Bind::Null`
/// without needing a separate raw-sqlx path.
///
/// Returns the row directly (no re-fetch round-trip) since we know all values.
pub async fn create(
    pool: &SqlitePool,
    name: &str,
    folder_path: &str,
    color: Option<&str>,
) -> sqlx::Result<WorkspaceRow> {
    // Allocate each owned value once, then clone into the bind array — avoids a
    // second allocation per field when constructing the returned row.
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let name = name.to_owned();
    let folder_path = folder_path.to_owned();
    let color = color.map(str::to_owned);

    QueryBuilder::<Sqlite>::table("workspace")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("name", Bind::Text(name.clone())),
            ("folder_path", Bind::Text(folder_path.clone())),
            // Option<String> → Value::Text(s) | Value::Null (no raw sqlx needed)
            ("color", color.clone().map(Bind::Text).unwrap_or(Bind::Null)),
            ("created_at", Bind::Text(created_at.clone())),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(WorkspaceRow {
        id,
        name,
        folder_path,
        color,
        created_at,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    /// Create a workspace and verify every field round-trips through the DB.
    /// Tests both Some(color) and None color.
    #[tokio::test]
    async fn create_then_get_roundtrip() {
        let pool = connect_in_memory().await;

        // With color
        let row = create(&pool, "My Workspace", "/tmp/my-ws", Some("#ff0000"))
            .await
            .expect("create with color failed");
        assert_eq!(row.name, "My Workspace");
        assert_eq!(row.folder_path, "/tmp/my-ws");
        assert_eq!(row.color.as_deref(), Some("#ff0000"));
        assert!(!row.id.is_empty());
        assert!(!row.created_at.is_empty());

        let fetched = get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched, row); // WorkspaceRow: PartialEq — full-row round-trip

        // None color round-trips to None
        let no_color = create(&pool, "Plain", "/tmp/plain", None)
            .await
            .expect("create without color failed");
        assert!(no_color.color.is_none());
        let fetched2 = get(&pool, &no_color.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert!(fetched2.color.is_none());
    }

    /// Creating 2 workspaces → list() returns 2, ordered by created_at.
    #[tokio::test]
    async fn list_returns_created() {
        let pool = connect_in_memory().await;

        // Empty DB starts empty
        assert_eq!(list(&pool).await.expect("list failed").len(), 0);

        create(&pool, "Alpha", "/a", None)
            .await
            .expect("create Alpha");
        create(&pool, "Beta", "/b", Some("#00f"))
            .await
            .expect("create Beta");

        let rows = list(&pool).await.expect("list failed");
        assert_eq!(rows.len(), 2);
        // Both names present (order may vary by created_at precision)
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"Alpha"));
        assert!(names.contains(&"Beta"));
    }

    /// exists() returns true for a known id, false for an unknown one.
    #[tokio::test]
    async fn exists_true_false() {
        let pool = connect_in_memory().await;

        assert!(!exists(&pool, "no-such-id").await.expect("exists failed"));

        let row = create(&pool, "Test", "/t", None)
            .await
            .expect("create failed");
        assert!(exists(&pool, &row.id).await.expect("exists failed"));
        assert!(!exists(&pool, "wrong-id").await.expect("exists failed"));
    }

    /// Serialized JSON must use camelCase keys — this locks the TS Workspace contract.
    #[tokio::test]
    async fn create_serializes_camel_case() {
        let pool = connect_in_memory().await;
        let row = create(&pool, "CamelTest", "/cam", Some("#123"))
            .await
            .expect("create failed");

        let json = serde_json::to_value(&row).expect("serialize failed");
        assert!(json.get("folderPath").is_some(), "must have folderPath key");
        assert!(json.get("createdAt").is_some(), "must have createdAt key");
        assert!(
            json.get("folder_path").is_none(),
            "must NOT have snake_case folder_path"
        );
        assert!(
            json.get("created_at").is_none(),
            "must NOT have snake_case created_at"
        );
    }
}

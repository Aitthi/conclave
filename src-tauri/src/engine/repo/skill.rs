//! Skill repository — reusable prompt modules attached to `AgentDefinition`s.
//! Mirrors `agent_definition.rs`'s pattern. Builtin-vs-custom AUTHORIZATION
//! (rejecting an edit/delete of a `kind = 'builtin'` row) is enforced at the
//! COMMAND layer (`commands::skill`), not here — this module is a plain CRUD
//! mirror, same division of responsibility as `agent_definition`'s
//! `permission_mode` allowlist check living in `commands::agent`, not the repo.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite, Value as Bind};
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Decoded row from the `skill` table. `kind` is `"builtin"` or `"custom"`
/// (DB CHECK enforces this). `content` is always present (defaults to `""`
/// at the schema level, never NULL).
#[allow(dead_code)] // consumed by Task 8's command handlers
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SkillRow {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub content: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

const COLS: [&str; 6] = ["id", "name", "description", "content", "kind", "icon"];

/// All skills, builtin first. Relies on `'builtin' < 'custom'` in ASCII/UTF-8
/// ordering (both are lowercase ASCII words compared lexically) — not an
/// arbitrary CASE expression, but real and worth a comment for the next reader.
#[allow(dead_code)]
pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<SkillRow>> {
    QueryBuilder::<Sqlite>::table("skill")
        .select(COLS)
        .order_by("kind", Order::Asc)
        .order_by("name", Order::Asc)
        .fetch_all::<SkillRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Skills of exactly one kind (`"builtin"` or `"custom"`), ordered by `id` for
/// determinism — used by `content_for_agent` to fetch every builtin
/// unconditionally, and by `commands::agent::save` to validate requested
/// custom skill ids.
#[allow(dead_code)]
pub async fn list_by_kind(pool: &SqlitePool, kind: &str) -> sqlx::Result<Vec<SkillRow>> {
    QueryBuilder::<Sqlite>::table("skill")
        .select(COLS)
        .where_eq("kind", kind)
        .order_by("id", Order::Asc)
        .fetch_all::<SkillRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Fetch a single skill by `id`, or `None` if it does not exist.
#[allow(dead_code)]
pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<SkillRow>> {
    QueryBuilder::<Sqlite>::table("skill")
        .select(COLS)
        .where_eq("id", id)
        .fetch_optional::<SkillRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Create a new CUSTOM skill (builtin rows are seed-only, never created via
/// this path) and return the constructed row.
#[allow(dead_code)]
pub async fn create(
    pool: &SqlitePool,
    name: &str,
    description: Option<&str>,
    content: &str,
) -> sqlx::Result<SkillRow> {
    let id = Uuid::new_v4().to_string();

    QueryBuilder::<Sqlite>::table("skill")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("name", Bind::Text(name.to_owned())),
            (
                "description",
                description
                    .map(|d| Bind::Text(d.to_owned()))
                    .unwrap_or(Bind::Null),
            ),
            ("content", Bind::Text(content.to_owned())),
            ("kind", Bind::Text("custom".to_owned())),
            ("icon", Bind::Null),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(SkillRow {
        id,
        name: name.to_owned(),
        description: description.map(str::to_owned),
        content: content.to_owned(),
        kind: "custom".to_owned(),
        icon: None,
    })
}

/// Update an existing skill's mutable fields and return the updated row, or
/// `None` if no row with `id` exists. Does NOT check `kind` — a caller wanting
/// to protect builtin rows from mutation must check `get()`'s result first
/// (see `commands::skill::save`).
#[allow(dead_code)]
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    description: Option<&str>,
    content: &str,
) -> sqlx::Result<Option<SkillRow>> {
    QueryBuilder::<Sqlite>::table("skill")
        .update([
            ("name", Bind::Text(name.to_owned())),
            (
                "description",
                description
                    .map(|d| Bind::Text(d.to_owned()))
                    .unwrap_or(Bind::Null),
            ),
            ("content", Bind::Text(content.to_owned())),
        ])
        .where_eq("id", id)
        .execute(pool)
        .await
        .map_err(cb_err)?;

    get(pool, id).await
}

/// Delete a skill. Returns `true` if a row was deleted. `agent_skill` rows
/// referencing it cascade away (`ON DELETE CASCADE`, `0001_init.sql:144`) —
/// no further cleanup needed. Does NOT check `kind`; see `update`'s note.
#[allow(dead_code)]
pub async fn delete(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    let res = sqlx::query("DELETE FROM skill WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use crate::engine::db::connect_in_memory;

    #[tokio::test]
    async fn create_then_get_roundtrip() {
        let pool = connect_in_memory().await;
        let row = super::create(
            &pool,
            "Reviewer",
            Some("Reviews diffs"),
            "Always check for X",
        )
        .await
        .expect("create failed");
        assert_eq!(row.name, "Reviewer");
        assert_eq!(row.description.as_deref(), Some("Reviews diffs"));
        assert_eq!(row.content, "Always check for X");
        assert_eq!(row.kind, "custom");
        assert!(row.icon.is_none());

        let fetched = super::get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched, row);
    }

    #[tokio::test]
    async fn create_with_no_description_stores_none() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Bare", None, "content")
            .await
            .expect("create failed");
        assert!(row.description.is_none());
    }

    #[tokio::test]
    async fn update_changes_fields() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Old", Some("old desc"), "old content")
            .await
            .expect("create failed");

        let updated = super::update(&pool, &row.id, "New", Some("new desc"), "new content")
            .await
            .expect("update failed")
            .expect("row should exist after update");
        assert_eq!(updated.name, "New");
        assert_eq!(updated.description.as_deref(), Some("new desc"));
        assert_eq!(updated.content, "new content");
    }

    #[tokio::test]
    async fn update_unknown_id_returns_none() {
        let pool = connect_in_memory().await;
        let result = super::update(&pool, "no-such-id", "X", None, "Y")
            .await
            .expect("update should not error");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn delete_removes_then_second_is_noop() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Gone", None, "c")
            .await
            .expect("create failed");
        assert!(super::delete(&pool, &row.id).await.expect("delete failed"));
        assert!(super::get(&pool, &row.id)
            .await
            .expect("get failed")
            .is_none());
        assert!(!super::delete(&pool, &row.id)
            .await
            .expect("second delete failed"));
    }

    #[tokio::test]
    async fn list_orders_builtin_before_custom() {
        let pool = connect_in_memory().await;
        // Seed a builtin row directly (create() only ever makes custom rows).
        sqlx::query("INSERT INTO skill (id, name, kind) VALUES ('sk-builtin', 'Core', 'builtin')")
            .execute(&pool)
            .await
            .expect("seed builtin failed");
        super::create(&pool, "Custom", None, "c")
            .await
            .expect("create failed");

        let all = super::list(&pool).await.expect("list failed");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].kind, "builtin");
        assert_eq!(all[1].kind, "custom");
    }

    #[tokio::test]
    async fn list_by_kind_filters() {
        let pool = connect_in_memory().await;
        sqlx::query("INSERT INTO skill (id, name, kind) VALUES ('sk-b', 'Core', 'builtin')")
            .execute(&pool)
            .await
            .expect("seed failed");
        super::create(&pool, "Custom", None, "c")
            .await
            .expect("create failed");

        let builtins = super::list_by_kind(&pool, "builtin")
            .await
            .expect("list failed");
        assert_eq!(builtins.len(), 1);
        assert_eq!(builtins[0].id, "sk-b");

        let customs = super::list_by_kind(&pool, "custom")
            .await
            .expect("list failed");
        assert_eq!(customs.len(), 1);
    }

    /// JSON must use camelCase keys and omit `description`/`icon` when None.
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Sol", None, "content")
            .await
            .expect("create failed");
        let json = serde_json::to_value(&row).expect("serialize failed");
        assert!(json.get("kind").is_some());
        assert!(json.get("content").is_some());
        assert!(json.get("description").is_none(), "None must be omitted");
        assert!(json.get("icon").is_none(), "None must be omitted");
    }
}

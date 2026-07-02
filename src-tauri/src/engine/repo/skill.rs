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
use std::collections::HashMap;
use uuid::Uuid;

/// A skill as seen by callers — either a builtin (from a bundled folder, see
/// `list_builtin`) or a custom one (a DB row). `kind` distinguishes them for
/// JSON output; nothing about this struct's SHAPE differs by kind.
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

/// Decoded row from the `skill` table, which (as of migration 0005) has no
/// `kind` column at all — every row is, structurally, a custom skill. Never
/// exposed outside this module; converted to `SkillRow` via `From` below.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
struct CustomSkillDbRow {
    id: String,
    name: String,
    description: Option<String>,
    content: String,
    icon: Option<String>,
}

impl From<CustomSkillDbRow> for SkillRow {
    fn from(r: CustomSkillDbRow) -> Self {
        SkillRow {
            id: r.id,
            name: r.name,
            description: r.description,
            content: r.content,
            kind: "custom".to_owned(),
            icon: r.icon,
        }
    }
}

const COLS: [&str; 5] = ["id", "name", "description", "content", "icon"];

/// All CUSTOM skills (the only kind stored in the DB), ordered by name.
pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<SkillRow>> {
    QueryBuilder::<Sqlite>::table("skill")
        .select(COLS)
        .order_by("name", Order::Asc)
        .fetch_all::<CustomSkillDbRow, _>(pool)
        .await
        .map_err(cb_err)
        .map(|rows| rows.into_iter().map(SkillRow::from).collect())
}

/// Fetch a single CUSTOM skill by `id`, or `None` if it does not exist. A
/// builtin id (never in the DB) correctly returns `None` here — callers
/// needing to distinguish "unknown id" from "builtin id" must check
/// `list_builtin()` first (see `commands::skill::save`/`delete`).
pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<SkillRow>> {
    QueryBuilder::<Sqlite>::table("skill")
        .select(COLS)
        .where_eq("id", id)
        .fetch_optional::<CustomSkillDbRow, _>(pool)
        .await
        .map_err(cb_err)
        .map(|opt| opt.map(SkillRow::from))
}

/// Create a new custom skill and return the constructed row.
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

/// Update an existing custom skill's mutable fields and return the updated
/// row, or `None` if no row with `id` exists. Does NOT check whether `id` is
/// a builtin — a builtin id is simply never in the DB, so `get(id)` after
/// this returns `None` for one; the command layer must reject a builtin
/// mutation attempt BEFORE calling this (see `commands::skill::save`).
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

/// Delete a custom skill. Returns `true` if a row was deleted. `agent_skill`
/// rows referencing it cascade away (`ON DELETE CASCADE`, `0001_init.sql:144`).
pub async fn delete(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    let res = sqlx::query("DELETE FROM skill WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// All skills attached to `agent_def_id` via `agent_skill`, ordered by
/// `sort_order` then `id` as a stable tie-breaker. `agent_skill` only ever
/// references CUSTOM (DB) skills — builtins are never attached via this
/// table, see `content_for_agent`.
pub async fn attached_to_agent(
    pool: &SqlitePool,
    agent_def_id: &str,
) -> sqlx::Result<Vec<SkillRow>> {
    let rows: Vec<CustomSkillDbRow> = sqlx::query_as(
        "SELECT s.id, s.name, s.description, s.content, s.icon \
         FROM agent_skill a JOIN skill s ON s.id = a.skill_id \
         WHERE a.agent_def_id = ? \
         ORDER BY a.sort_order ASC, s.id ASC",
    )
    .bind(agent_def_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(SkillRow::from).collect())
}

/// Replace an agent definition's CUSTOM skill attachments with exactly
/// `skill_ids`, in that order (index becomes `sort_order`). Delete-then-insert
/// inside one transaction. An empty slice clears all attachments.
pub async fn set_custom_attachments(
    pool: &SqlitePool,
    agent_def_id: &str,
    skill_ids: &[String],
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM agent_skill WHERE agent_def_id = ?")
        .bind(agent_def_id)
        .execute(&mut *tx)
        .await?;

    for (idx, skill_id) in skill_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO agent_skill (agent_def_id, skill_id, sort_order) VALUES (?, ?, ?)",
        )
        .bind(agent_def_id)
        .bind(skill_id)
        .bind(idx as i64)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Every agent definition's custom skill ids, grouped by `agent_def_id` and
/// ordered by `sort_order` within each group. Used by `commands::agent::list`.
pub async fn custom_skill_ids_by_agent(
    pool: &SqlitePool,
) -> sqlx::Result<HashMap<String, Vec<String>>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT agent_def_id, skill_id FROM agent_skill ORDER BY agent_def_id, sort_order",
    )
    .fetch_all(pool)
    .await?;

    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (agent_def_id, skill_id) in rows {
        map.entry(agent_def_id).or_default().push(skill_id);
    }
    Ok(map)
}

/// Count of `agent_skill` rows per skill id. Used by `commands::skill::list`
/// for the Library's "attached to N agents" label (custom skills only).
pub async fn attached_counts(pool: &SqlitePool) -> sqlx::Result<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT skill_id, COUNT(*) FROM agent_skill GROUP BY skill_id")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
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
        assert_eq!(updated.kind, "custom");
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
    async fn list_returns_only_custom_ordered_by_name() {
        let pool = connect_in_memory().await;
        super::create(&pool, "Zeta", None, "z")
            .await
            .expect("create failed");
        super::create(&pool, "Alpha", None, "a")
            .await
            .expect("create failed");

        let all = super::list(&pool).await.expect("list failed");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "Alpha", "must be name-ordered");
        assert_eq!(all[1].name, "Zeta");
        assert!(all.iter().all(|s| s.kind == "custom"));
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

    async fn fixture_agent_def(pool: &sqlx::SqlitePool) -> String {
        crate::engine::repo::agent_definition::create(
            pool,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "Fixture".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed")
        .id
    }

    #[tokio::test]
    async fn attached_to_agent_respects_sort_order() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "First", None, "1")
            .await
            .expect("create failed");
        let s2 = super::create(&pool, "Second", None, "2")
            .await
            .expect("create failed");

        // Attach in REVERSE order to prove sort_order (not insertion order) wins.
        super::set_custom_attachments(&pool, &def_id, &[s2.id.clone(), s1.id.clone()])
            .await
            .expect("set_custom_attachments failed");

        let attached = super::attached_to_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert_eq!(attached.len(), 2);
        assert_eq!(attached[0].id, s2.id, "sort_order 0 must come first");
        assert_eq!(attached[1].id, s1.id);
    }

    #[tokio::test]
    async fn set_custom_attachments_replaces_not_appends() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "A", None, "a")
            .await
            .expect("create failed");
        let s2 = super::create(&pool, "B", None, "b")
            .await
            .expect("create failed");

        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&s1.id))
            .await
            .expect("first set failed");
        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&s2.id))
            .await
            .expect("second set failed");

        let attached = super::attached_to_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert_eq!(attached.len(), 1, "second call must REPLACE, not append");
        assert_eq!(attached[0].id, s2.id);
    }

    #[tokio::test]
    async fn set_custom_attachments_empty_clears() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "A", None, "a")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");
        super::set_custom_attachments(&pool, &def_id, &[])
            .await
            .expect("clear failed");

        let attached = super::attached_to_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert!(attached.is_empty());
    }

    #[tokio::test]
    async fn custom_skill_ids_by_agent_groups_correctly() {
        let pool = connect_in_memory().await;
        let def1 = fixture_agent_def(&pool).await;
        let def2 = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "A", None, "a")
            .await
            .expect("create failed");
        let s2 = super::create(&pool, "B", None, "b")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def1, &[s1.id.clone(), s2.id.clone()])
            .await
            .expect("set failed");
        super::set_custom_attachments(&pool, &def2, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");

        let map = super::custom_skill_ids_by_agent(&pool)
            .await
            .expect("query failed");
        assert_eq!(map.get(&def1).cloned().unwrap_or_default().len(), 2);
        assert_eq!(map.get(&def2).cloned().unwrap_or_default(), vec![s1.id]);
    }

    #[tokio::test]
    async fn attached_counts_counts_across_agents() {
        let pool = connect_in_memory().await;
        let def1 = fixture_agent_def(&pool).await;
        let def2 = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "Shared", None, "s")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def1, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");
        super::set_custom_attachments(&pool, &def2, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");

        let counts = super::attached_counts(&pool).await.expect("query failed");
        assert_eq!(counts.get(&s1.id).copied(), Some(2));
    }

    #[tokio::test]
    async fn delete_skill_cascades_agent_skill_rows() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "Doomed", None, "d")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");

        super::delete(&pool, &s1.id).await.expect("delete failed");

        let attached = super::attached_to_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert!(attached.is_empty(), "agent_skill row must cascade away");
        assert!(
            crate::engine::repo::agent_definition::exists(&pool, &def_id)
                .await
                .expect("exists check failed"),
            "agent_definition itself must be untouched"
        );
    }
}

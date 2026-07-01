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

use std::collections::HashMap;

/// All skills attached to `agent_def_id` via `agent_skill`, ordered by
/// `sort_order` then `id` as a stable tie-breaker. Builtin skills are NEVER
/// stored here — see `content_for_agent`, which fetches them separately and
/// unconditionally.
#[allow(dead_code)]
pub async fn attached_to_agent(
    pool: &SqlitePool,
    agent_def_id: &str,
) -> sqlx::Result<Vec<SkillRow>> {
    sqlx::query_as::<_, SkillRow>(
        "SELECT s.id, s.name, s.description, s.content, s.kind, s.icon \
         FROM agent_skill a JOIN skill s ON s.id = a.skill_id \
         WHERE a.agent_def_id = ? \
         ORDER BY a.sort_order ASC, s.id ASC",
    )
    .bind(agent_def_id)
    .fetch_all(pool)
    .await
}

/// Replace an agent definition's CUSTOM skill attachments with exactly
/// `skill_ids`, in that order (index becomes `sort_order`). Delete-then-insert
/// inside one transaction, mirroring `workspace_agent::instantiate`'s
/// transaction style. An empty slice clears all attachments.
#[allow(dead_code)]
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
/// ordered by `sort_order` within each group. One query for ALL definitions —
/// used by `commands::agent::list` to annotate `AgentDefinition.skillIds`
/// without an N+1 query per definition.
#[allow(dead_code)]
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

/// Count of `agent_skill` rows per skill id — how many agent definitions have
/// each CUSTOM skill attached (a builtin skill has no `agent_skill` rows at
/// all, so it's simply absent from the map / reads as 0). Used by
/// `commands::skill::list` for the Library's "attached to N agents" label.
#[allow(dead_code)]
pub async fn attached_counts(pool: &SqlitePool) -> sqlx::Result<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT skill_id, COUNT(*) FROM agent_skill GROUP BY skill_id")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

/// Build the concatenated skill body for one agent definition's `cli` launch,
/// plus the ordered list of skill ids used (for `session.launched_skill_ids`).
/// Builtin skills come first (fixed `id` order, via `list_by_kind`), then
/// custom skills by `sort_order` (via `attached_to_agent`). Each skill renders
/// as a `## Skill: {name}` header followed by its `content`, sections
/// separated by a blank line. Returns `("", [])` when nothing is attached and
/// no builtin skills exist — the caller (`commands::instance::spawn`) treats
/// an empty body as "skip the sidecar file entirely".
#[allow(dead_code)] // consumed by Task 10's commands::instance::spawn
pub async fn content_for_agent(
    pool: &SqlitePool,
    agent_def_id: &str,
) -> sqlx::Result<(String, Vec<String>)> {
    let builtins = list_by_kind(pool, "builtin").await?;
    let customs = attached_to_agent(pool, agent_def_id).await?;

    let mut ids = Vec::with_capacity(builtins.len() + customs.len());
    let mut sections = Vec::with_capacity(builtins.len() + customs.len());
    for s in builtins.iter().chain(customs.iter()) {
        ids.push(s.id.clone());
        sections.push(format!("## Skill: {}\n\n{}", s.name, s.content));
    }
    Ok((sections.join("\n\n"), ids))
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

    /// Deleting a skill cascades its `agent_skill` rows without touching the
    /// agent_definition (locks in the ON DELETE CASCADE relied on in the ADR).
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

    #[tokio::test]
    async fn content_for_agent_orders_builtin_then_custom_with_headers() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        sqlx::query("INSERT INTO skill (id, name, content, kind) VALUES ('sk-b', 'Base', 'Be careful', 'builtin')")
            .execute(&pool)
            .await
            .expect("seed builtin failed");
        let custom = super::create(&pool, "Extra", None, "Do X")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&custom.id))
            .await
            .expect("set failed");

        let (body, ids) = super::content_for_agent(&pool, &def_id)
            .await
            .expect("query failed");

        assert_eq!(
            ids,
            vec!["sk-b".to_string(), custom.id.clone()],
            "builtin must come first"
        );
        let base_pos = body.find("## Skill: Base").expect("Base header missing");
        let extra_pos = body.find("## Skill: Extra").expect("Extra header missing");
        assert!(
            base_pos < extra_pos,
            "builtin section must precede custom section"
        );
        assert!(body.contains("Be careful"));
        assert!(body.contains("Do X"));
    }

    #[tokio::test]
    async fn content_for_agent_empty_when_nothing_attached() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let (body, ids) = super::content_for_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert_eq!(body, "");
        assert!(ids.is_empty());
    }
}

# Builtin Skills From Bundled Folder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace DB-seeded builtin skills (`skill.kind = 'builtin'`) with `SKILL.md` files bundled into the app at build time, mirroring Claude Code's own skill format — with zero frontend changes.

**Architecture:** The `skill` DB table becomes custom-only (the `kind` column is dropped). A new sync, infallible `repo::skill::list_builtin()` reads `<skill-folder>/SKILL.md` files from a directory resolved relative to the running executable (bundled `.app` Resources dir in production, the source tree in dev/test — mirroring `agentctx::ensure_conclave_shim`'s existing exe-relative pattern), constructing `SkillRow { kind: "builtin", .. }` values that never touch the database. Every call site that used to fetch DB-seeded builtins now calls this instead.

**Tech Stack:** Rust (sqlx + chain-builder, SQLite), Tauri v2 bundle resources.

## Global Constraints

- Rust baseline: `cargo test --manifest-path src-tauri/Cargo.toml --lib` + `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` + `cargo fmt --manifest-path src-tauri/Cargo.toml --check`.
- Frontend baseline (confirm nothing broke — no frontend code changes expected in this plan): `pnpm exec tsc --noEmit && pnpm build`.
- No new dependency for frontmatter parsing — hand-roll it (the format is two flat string fields, not general YAML). No `tempfile` crate — use `std::env::temp_dir()` directly, mirroring `agentctx.rs`'s existing sidecar test.
- Commits authored as `detoro <meanstack20@gmail.com>` with a `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer.
- Read `docs/adr/0002-builtin-skills-from-bundled-folder.md` for the "why" behind these decisions.
- This plan follows on top of the already-shipped Skill System v1 (commits `3f88451..17b84e1` on `main`).

---

### Task 1: Migration 0005 — drop `skill.kind`

**Files:**
- Create: `src-tauri/src/engine/migrations/0005_drop_skill_kind.sql`
- Modify: `src-tauri/src/engine/db.rs` (add `if version < 5 { ... }` gate; rewrite `migrate_adds_skill_system_columns`; add `migrate_drops_skill_kind_column`)

**Interfaces:**
- Produces: `skill` table with NO `kind` column (verified via `PRAGMA table_info`) — every later task in this plan depends on this column being gone.

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/engine/db.rs`, REPLACE the existing `migrate_adds_skill_system_columns` test (it currently inserts a `kind` column and asserts `user_version == 4`, both of which become wrong once this migration lands) with:

```rust
    /// Migration 0004 added `skill.content` and `session.launched_skill_ids`
    /// (kind was later dropped by migration 0005 — see the next test).
    #[tokio::test]
    async fn migrate_adds_skill_system_columns() {
        let pool = connect_in_memory().await;

        sqlx::query("INSERT INTO skill (id, name) VALUES ('sk1', 'Test')")
            .execute(&pool)
            .await
            .expect("insert should succeed");

        let content: String = sqlx::query_scalar("SELECT content FROM skill WHERE id = 'sk1'")
            .fetch_one(&pool)
            .await
            .expect("select should succeed");
        assert_eq!(content, "");

        // session.launched_skill_ids exists and defaults to NULL.
        let ws = crate::engine::repo::workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = crate::engine::repo::agent_definition::create(
            &pool,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "A".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        let wa = crate::engine::repo::workspace_agent::create(&pool, &ws.id, &def.id, "idle")
            .await
            .expect("create workspace_agent failed");
        let session = crate::engine::repo::session::create_for_instance(&pool, &wa.id)
            .await
            .expect("create session failed");
        let launched: Option<String> =
            sqlx::query_scalar("SELECT launched_skill_ids FROM session WHERE id = ?")
                .bind(&session.id)
                .fetch_one(&pool)
                .await
                .expect("select should succeed");
        assert!(launched.is_none());

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("pragma read failed");
        assert_eq!(version, 5);
    }

    /// Migration 0005 drops `skill.kind` entirely — builtin skills now come
    /// from a bundled folder, never the DB (see ADR 0002). Every `skill` row
    /// after this migration is, structurally, a custom skill.
    #[tokio::test]
    async fn migrate_drops_skill_kind_column() {
        let pool = connect_in_memory().await;

        let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('skill')")
            .fetch_all(&pool)
            .await
            .expect("pragma_table_info query failed");
        assert!(
            !columns.iter().any(|c| c == "kind"),
            "skill.kind must not exist after migration: {columns:?}"
        );

        // An insert with no `kind` column reference must succeed.
        sqlx::query("INSERT INTO skill (id, name, content) VALUES ('sk-no-kind', 'X', 'body')")
            .execute(&pool)
            .await
            .expect("insert without kind should succeed");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::db::tests`
Expected: FAIL — `migrate_adds_skill_system_columns` fails its `user_version == 5` assertion (currently 4), and `migrate_drops_skill_kind_column` fails to compile/run (migration 0005 doesn't exist yet, `kind` still present).

- [ ] **Step 3: Write the migration file**

```sql
-- Builtin skills now come from a bundled `skills/` folder, never the DB (see
-- docs/adr/0002-builtin-skills-from-bundled-folder.md) — every remaining
-- `skill` row is, structurally, a user-authored custom skill. SQLite 3.35+
-- supports DROP COLUMN for a plain column with a self-referencing CHECK
-- constraint (kind's CHECK only referenced kind itself, not another column).
ALTER TABLE skill DROP COLUMN kind;
```

- [ ] **Step 4: Wire the migration into `migrate()`**

In `src-tauri/src/engine/db.rs`, after the `if version < 4 { ... }` block, before the closing `tx.commit().await?;`:

```rust
    if version < 5 {
        sqlx::raw_sql(include_str!("migrations/0005_drop_skill_kind.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 5;")
            .execute(&mut *tx)
            .await?;
    }

```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::db::tests`
Expected: PASS (both tests, plus the pre-existing `migrate_creates_all_tables`/`migrate_is_idempotent`/`migrate_seeds_core_conclave_tool` tests still pass — table count is unaffected, only a column was dropped).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/migrations/0005_drop_skill_kind.sql src-tauri/src/engine/db.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(db): migration 0005 drops skill.kind — builtins now come from a folder

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `repo::skill` — custom-only DB layer (`CustomSkillDbRow`)

**Files:**
- Modify: `src-tauri/src/engine/repo/skill.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `list(pool) -> Vec<SkillRow>` (all custom, name-ordered), `get(pool, id) -> Option<SkillRow>`, `create`/`update`/`delete` unchanged signatures — `list_by_kind` is REMOVED (Task 3-5 must stop calling it).

- [ ] **Step 1: Write the failing tests**

Because `list_by_kind` is being removed and every test seeding a `kind='builtin'` row via raw SQL will fail to compile (no `kind` column), REPLACE the ENTIRE test module at the bottom of `skill.rs` with this version (same tests, minus the ones that only existed to test `list_by_kind`/builtin-via-SQL, which move to Task 3):

```rust
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
```

(Note: `content_for_agent`'s tests move to Task 3, once `list_builtin()` exists — they can't compile yet with the old `list_by_kind`-based version. This task's replacement test module intentionally drops them for now.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: FAIL to compile — `SkillRow`/queries still reference a `kind` DB column that Task 1 already dropped, so every query against `COLS` (which still includes `"kind"`) will fail at runtime, and `content_for_agent`'s old body (still calling `list_by_kind`, which you're about to delete) breaks compilation once you delete it in the next step. Proceed to Step 3 regardless — this is the expected broken intermediate state.

- [ ] **Step 3: Rewrite the DB-facing half of `skill.rs`**

Replace the file's content from the top (`use super::cb_err;`) down through the end of `content_for_agent` (i.e. everything before `#[cfg(test)]`) with:

```rust
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
```

(`content_for_agent`, `list_builtin`, `parse_skill_md`, and their supporting `skills_dir`/`bundled_skills_dir` helpers are added in Task 3 — do NOT add them here, this task is DB-layer only.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: PASS (13 tests) — note `content_for_agent` no longer exists at this point in the file; that's expected, Task 3 restores it.

- [ ] **Step 5: Run the full crate to confirm the expected remaining breakage is ONLY in files Task 3-5 will fix**

Run: `cargo build --manifest-path src-tauri/Cargo.toml --lib 2>&1 | head -60`
Expected: compile errors in `src-tauri/src/engine/repo/skill.rs`'s own removed `content_for_agent` callers — specifically `commands/instance.rs` (calls `repo::skill::content_for_agent`), `commands/skill.rs` (calls the old `repo::skill::list`, still fine — signature unchanged — but also references removed `list_by_kind` indirectly via nothing; check), and `commands/agent.rs` (calls `repo::skill::list_by_kind`, which no longer exists). This is EXPECTED — Tasks 3-5 fix these call sites. Do not attempt to fix them in this task; just confirm the error list matches this description (only `list_by_kind`/`content_for_agent` callers, nothing else) before committing.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/repo/skill.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(repo): skill.rs becomes custom-only (CustomSkillDbRow, no kind column)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

Note: this commit intentionally leaves the crate NOT compiling as a whole (callers of the now-removed `list_by_kind`/`content_for_agent` are fixed in Tasks 3-5). This mirrors the original Skill System plan's own precedent of committing a repo-layer change before its command-layer callers catch up — each task's own module tests (Step 4 above) pass in isolation via `cargo test --lib engine::repo::skill`, which is what's verified here.

---

### Task 3: `list_builtin`, `parse_skill_md`, `content_for_agent`, and the example fixture

**Files:**
- Modify: `src-tauri/src/engine/repo/skill.rs`
- Create: `src-tauri/skills/example/SKILL.md`

**Interfaces:**
- Consumes: `SkillRow`, `attached_to_agent` (Task 2).
- Produces: `list_builtin() -> Vec<SkillRow>` (sync, infallible), `content_for_agent(pool, agent_def_id) -> (String, Vec<String>)` — consumed by Task 4 (`commands::skill.rs`), Task 5 (`commands::agent.rs`), and the already-shipped `commands::instance.rs` (unchanged call site, just needs this function to exist again).

- [ ] **Step 1: Create the example skill fixture**

```markdown
---
name: Example Skill
description: Demonstrates the builtin skill file format — safe to remove or replace.
---

This is an example builtin skill shipped with Conclave to demonstrate the
file format (see docs/adr/0002-builtin-skills-from-bundled-folder.md). Each
builtin skill is a subdirectory of `skills/` containing exactly one
`SKILL.md` file: two frontmatter fields (`name`, `description`) between
`---` markers, followed by the skill's full instructional content as
ordinary markdown.
```

Write this to `src-tauri/skills/example/SKILL.md`. This is both the real shipped mechanism AND the deterministic test fixture every test below relies on — `list_builtin()`'s dev/test fallback resolves to `CARGO_MANIFEST_DIR/skills` (i.e. `src-tauri/skills`), which `cargo test` always has access to regardless of working directory.

- [ ] **Step 2: Write the failing tests**

Add to `skill.rs`'s `mod tests` block (from Task 2):

```rust
    #[test]
    fn parse_skill_md_extracts_frontmatter_and_body() {
        let raw = "---\nname: Reviewer\ndescription: Reviews diffs\n---\n\nAlways check X.\n";
        let (name, description, content) =
            super::parse_skill_md(raw).expect("should parse");
        assert_eq!(name, "Reviewer");
        assert_eq!(description.as_deref(), Some("Reviews diffs"));
        assert_eq!(content, "Always check X.");
    }

    #[test]
    fn parse_skill_md_description_optional() {
        let raw = "---\nname: Bare\n---\n\nBody only.\n";
        let (name, description, content) = super::parse_skill_md(raw).expect("should parse");
        assert_eq!(name, "Bare");
        assert!(description.is_none());
        assert_eq!(content, "Body only.");
    }

    #[test]
    fn parse_skill_md_rejects_missing_frontmatter() {
        assert!(super::parse_skill_md("Just a body, no frontmatter.").is_none());
    }

    #[test]
    fn parse_skill_md_rejects_missing_name() {
        let raw = "---\ndescription: no name here\n---\n\nBody.\n";
        assert!(super::parse_skill_md(raw).is_none());
    }

    /// Real filesystem test (no tempfile crate in this codebase — mirrors
    /// `agentctx.rs`'s existing sidecar test pattern of writing to a real
    /// path under `std::env::temp_dir()` and cleaning up manually).
    #[test]
    fn read_builtin_skills_from_parses_one_skill_per_subdir_skips_bad_ones() {
        let dir = std::env::temp_dir().join("conclave-skill-test-read-builtin-skills");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("good")).expect("mkdir failed");
        std::fs::write(
            dir.join("good").join("SKILL.md"),
            "---\nname: Good One\ndescription: Works\n---\n\nDo the thing.\n",
        )
        .expect("write failed");
        // A subdir with no SKILL.md at all must be silently skipped.
        std::fs::create_dir_all(dir.join("empty-dir")).expect("mkdir failed");
        // A subdir with unparsable frontmatter must be silently skipped.
        std::fs::create_dir_all(dir.join("bad")).expect("mkdir failed");
        std::fs::write(dir.join("bad").join("SKILL.md"), "no frontmatter here")
            .expect("write failed");
        // A plain FILE directly under dir (not a subdirectory) must be ignored.
        std::fs::write(dir.join("stray.txt"), "ignore me").expect("write failed");

        let skills = super::read_builtin_skills_from(&dir);

        assert_eq!(skills.len(), 1, "only the well-formed 'good' skill should survive");
        assert_eq!(skills[0].id, "good");
        assert_eq!(skills[0].name, "Good One");
        assert_eq!(skills[0].description.as_deref(), Some("Works"));
        assert_eq!(skills[0].content, "Do the thing.");
        assert_eq!(skills[0].kind, "builtin");

        std::fs::remove_dir_all(&dir).expect("cleanup failed");
    }

    #[test]
    fn read_builtin_skills_from_missing_dir_returns_empty() {
        let dir = std::env::temp_dir().join("conclave-skill-test-does-not-exist-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(super::read_builtin_skills_from(&dir).is_empty());
    }

    /// The real, checked-in `src-tauri/skills/example/` fixture must be
    /// discoverable via the production `list_builtin()` entry point in a
    /// `cargo test` run (its CARGO_MANIFEST_DIR fallback resolves to
    /// `src-tauri/skills` regardless of CWD).
    #[test]
    fn list_builtin_finds_the_checked_in_example_skill() {
        let skills = super::list_builtin();
        let example = skills
            .iter()
            .find(|s| s.id == "example")
            .expect("the checked-in example skill must be discoverable");
        assert_eq!(example.name, "Example Skill");
        assert_eq!(example.kind, "builtin");
    }

    #[tokio::test]
    async fn content_for_agent_orders_builtin_then_custom_with_headers() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
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
            vec!["example".to_string(), custom.id.clone()],
            "builtin (the checked-in example) must come first"
        );
        let base_pos = body.find("## Skill: Example Skill").expect("Example header missing");
        let extra_pos = body.find("## Skill: Extra").expect("Extra header missing");
        assert!(base_pos < extra_pos, "builtin section must precede custom section");
        assert!(body.contains("Do X"));
    }

    #[tokio::test]
    async fn content_for_agent_still_includes_builtin_when_nothing_custom_attached() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let (body, ids) = super::content_for_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert_eq!(ids, vec!["example".to_string()]);
        assert!(body.contains("## Skill: Example Skill"));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: FAIL to compile — `parse_skill_md`, `read_builtin_skills_from`, `list_builtin`, `content_for_agent` don't exist yet.

- [ ] **Step 4: Write the implementation**

Add to `skill.rs`, above the `#[cfg(test)]` block (after `attached_counts`):

```rust
/// Every skill shipped with the app (see
/// docs/adr/0002-builtin-skills-from-bundled-folder.md). Sync and infallible
/// — a missing/unreadable directory just yields zero skills rather than
/// propagating an error, so one bad file can never take down a `cli` agent's
/// launch (which depends on `content_for_agent` succeeding).
pub fn list_builtin() -> Vec<SkillRow> {
    read_builtin_skills_from(&skills_dir())
}

/// Parse every skill in `dir` — each direct subdirectory containing a
/// `SKILL.md` file becomes one builtin `SkillRow` (`id` = the subdirectory's
/// name). A subdirectory with no `SKILL.md`, or one with unparsable
/// frontmatter, is silently skipped rather than failing the whole read.
fn read_builtin_skills_from(dir: &std::path::Path) -> Vec<SkillRow> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        let Ok(raw) = std::fs::read_to_string(path.join("SKILL.md")) else {
            continue;
        };
        let Some((name, description, content)) = parse_skill_md(&raw) else {
            continue;
        };
        out.push(SkillRow {
            id,
            name,
            description,
            content,
            kind: "builtin".to_owned(),
            icon: None,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Parse a `SKILL.md`'s `---`-delimited frontmatter (flat `key: value` lines
/// — only `name`/`description` recognized) and body. Hand-rolled rather than
/// pulling in a YAML crate: the format is two flat string fields, not
/// general YAML (see ADR 0002). Returns `None` (skip this skill) if the file
/// doesn't start with a frontmatter block or `name` is missing/blank.
fn parse_skill_md(raw: &str) -> Option<(String, Option<String>, String)> {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let rest = raw.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let after_closing = &rest[end + 4..];
    let body = after_closing
        .strip_prefix("\r\n")
        .or_else(|| after_closing.strip_prefix('\n'))
        .unwrap_or(after_closing);

    let mut name = None;
    let mut description = None;
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("description:") {
            description = Some(v.trim().to_owned());
        }
    }
    let name = name.filter(|s| !s.is_empty())?;
    Some((
        name,
        description.filter(|s| !s.is_empty()),
        body.trim_end().to_owned(),
    ))
}

/// Resolve the real builtin-skills directory: the bundled `Resources/skills`
/// sibling of the running executable inside a packaged `.app` (mirrors
/// `agentctx::ensure_conclave_shim`'s exe-relative resolution for
/// `conclave-cli`), falling back to the source tree's `skills/` directory
/// (`CARGO_MANIFEST_DIR`, a compile-time constant) for a `cargo
/// run`/`cargo test`/`tauri dev` build, none of which have a `.app` bundle
/// structure.
fn skills_dir() -> std::path::PathBuf {
    if let Some(bundled) = bundled_skills_dir() {
        if bundled.is_dir() {
            return bundled;
        }
    }
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/skills"))
}

fn bundled_skills_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // macOS .app layout: Contents/MacOS/<exe> ; Contents/Resources/<...>
    Some(exe.parent()?.parent()?.join("Resources").join("skills"))
}

/// Build the concatenated skill body for one agent definition's `cli` launch,
/// plus the ordered list of skill ids used (for `session.launched_skill_ids`).
/// Builtin skills come first (fixed `id` order, via `list_builtin`), then
/// custom skills by `sort_order` (via `attached_to_agent`). Each skill renders
/// as a `## Skill: {name}` header followed by its `content`, sections
/// separated by a blank line.
pub async fn content_for_agent(
    pool: &SqlitePool,
    agent_def_id: &str,
) -> sqlx::Result<(String, Vec<String>)> {
    let builtins = list_builtin();
    let customs = attached_to_agent(pool, agent_def_id).await?;

    let mut ids = Vec::with_capacity(builtins.len() + customs.len());
    let mut sections = Vec::with_capacity(builtins.len() + customs.len());
    for s in builtins.iter().chain(customs.iter()) {
        ids.push(s.id.clone());
        sections.push(format!("## Skill: {}\n\n{}", s.name, s.content));
    }
    Ok((sections.join("\n\n"), ids))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: PASS (21 tests total)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/repo/skill.rs src-tauri/skills/example/SKILL.md
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(repo): read builtin skills from a bundled folder, ship an example

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `commands::skill` — wire builtin-from-folder into list/save/delete

**Files:**
- Modify: `src-tauri/src/engine/commands/skill.rs`

**Interfaces:**
- Consumes: `repo::skill::{list_builtin, list, attached_counts, get, create, update, delete}` (Tasks 2-3).

- [ ] **Step 1: Write the failing tests**

REPLACE `save_rejects_editing_builtin` and `delete_rejects_builtin_but_allows_custom` in `commands/skill.rs`'s test module (they currently seed a builtin via raw SQL, which no longer compiles — `kind` column is gone) with:

```rust
    #[tokio::test]
    async fn save_rejects_editing_builtin() {
        let state = AppState::for_tests().await;
        let builtin_id = repo::skill::list_builtin()
            .first()
            .expect("at least the checked-in example skill must exist")
            .id
            .clone();

        let result = save(
            &state,
            serde_json::json!({ "id": builtin_id, "name": "Hacked", "content": "x" }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Invalid(_))));
    }

    #[tokio::test]
    async fn delete_rejects_builtin_but_allows_custom() {
        let state = AppState::for_tests().await;
        let builtin_id = repo::skill::list_builtin()
            .first()
            .expect("at least the checked-in example skill must exist")
            .id
            .clone();
        let created = save(&state, serde_json::json!({ "name": "Custom", "content": "c" }))
            .await
            .expect("create failed");
        let custom_id = created["id"].as_str().unwrap().to_owned();

        let builtin_delete = delete(&state, serde_json::json!({ "id": builtin_id })).await;
        assert!(matches!(builtin_delete, Err(AppError::Invalid(_))));

        let custom_delete = delete(&state, serde_json::json!({ "id": custom_id })).await;
        assert!(custom_delete.is_ok());
    }

    #[tokio::test]
    async fn list_includes_builtin_and_custom() {
        let state = AppState::for_tests().await;
        save(&state, serde_json::json!({ "name": "Custom", "content": "c" }))
            .await
            .expect("create failed");

        let listed = list(&state, Value::Null).await.expect("list failed");
        let arr = listed.as_array().unwrap();
        assert!(
            arr.iter().any(|s| s["kind"] == "builtin" && s["id"] == "example"),
            "builtin example skill must appear in list()"
        );
        assert!(
            arr.iter().any(|s| s["kind"] == "custom" && s["name"] == "Custom"),
            "custom skill must appear in list()"
        );
        let builtin_item = arr.iter().find(|s| s["id"] == "example").unwrap();
        assert!(
            builtin_item.get("attachedTo").is_none(),
            "builtin items must not carry an attachedTo annotation"
        );
    }
```

Keep `save_creates_then_updates_custom_skill` and `list_annotates_attached_to_count` UNCHANGED (they never touched `kind`).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::skill`
Expected: FAIL — `save`/`delete` still call the old `repo::skill::get(id)`-then-check-`kind` pattern, which no longer distinguishes a builtin (never in the DB) from a truly unknown id, so a builtin id currently returns `NotFound` instead of `Invalid`; `list_includes_builtin_and_custom` fails since `list()` doesn't yet merge in builtins.

- [ ] **Step 3: Rewrite `list`/`save`/`delete`**

Replace the three handler functions (everything from `pub async fn list` through the end of `pub async fn delete`, i.e. lines 27-95 of the current file) with:

```rust
/// Return every skill (builtin first, from the bundled folder — see
/// `repo::skill::list_builtin` / ADR 0002), then every custom skill (from the
/// DB), each custom item annotated with `attachedTo`: how many agent
/// definitions currently have it attached. Builtin items carry no
/// `attachedTo` key at all (the frontend's system-skill card never reads it).
///
/// Maps to `skill.list` on the IPC bus.
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let builtins = repo::skill::list_builtin();
    let customs = repo::skill::list(&state.db).await?;
    let counts = repo::skill::attached_counts(&state.db).await?;

    let mut items = Vec::with_capacity(builtins.len() + customs.len());
    for s in &builtins {
        items.push(serde_json::to_value(s).map_err(|e| AppError::Internal(e.to_string()))?);
    }
    for s in &customs {
        let mut v = serde_json::to_value(s).map_err(|e| AppError::Internal(e.to_string()))?;
        v["attachedTo"] = serde_json::json!(counts.get(&s.id).copied().unwrap_or(0));
        items.push(v);
    }
    Ok(Value::Array(items))
}

/// Create or update a CUSTOM skill. Maps to `skill.save` on the IPC bus.
/// Rejects (`AppError::Invalid`) an attempt to edit a builtin skill — checked
/// against `repo::skill::list_builtin()` FIRST, since a builtin id is never
/// in the DB (a bare `repo::skill::get` would incorrectly report `NotFound`).
pub async fn save(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SaveSkillReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if let Some(id) = req.id.as_deref() {
        if repo::skill::list_builtin().iter().any(|s| s.id == id) {
            return Err(AppError::Invalid("cannot edit a builtin skill".into()));
        }
        let existing = repo::skill::get(&state.db, id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("skill id={id} not found")))?;
        let _ = existing; // presence already confirmed; row itself isn't needed further
        let row = repo::skill::update(
            &state.db,
            id,
            &req.name,
            req.description.as_deref(),
            &req.content,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("skill id={id} not found")))?;
        return serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()));
    }

    let row = repo::skill::create(
        &state.db,
        &req.name,
        req.description.as_deref(),
        &req.content,
    )
    .await?;
    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Delete a CUSTOM skill. Maps to `skill.delete` on the IPC bus. Rejects
/// (`AppError::Invalid`) an attempt to delete a builtin skill, checked
/// against `repo::skill::list_builtin()` first (same reasoning as `save`).
/// `agent_skill` rows referencing it cascade away (`ON DELETE CASCADE`).
pub async fn delete(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DeleteSkillReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if repo::skill::list_builtin().iter().any(|s| s.id == req.id) {
        return Err(AppError::Invalid("cannot delete a builtin skill".into()));
    }

    repo::skill::get(&state.db, &req.id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("skill id={} not found", req.id)))?;
    repo::skill::delete(&state.db, &req.id).await?;
    Ok(Value::Null)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::skill`
Expected: PASS (5 tests)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/engine/commands/skill.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(commands): skill.list/save/delete source builtins from the folder

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `commands::agent` — wire `list_builtin()` into `list`/`save`

**Files:**
- Modify: `src-tauri/src/engine/commands/agent.rs`

**Interfaces:**
- Consumes: `repo::skill::{list_builtin, list}` (Tasks 2-3).

- [ ] **Step 1: Write the failing test**

REPLACE `list_annotates_builtin_skill_ids_even_without_attachment` in `commands/agent.rs`'s test module (it currently seeds a builtin via raw SQL, which no longer compiles) with:

```rust
    #[tokio::test]
    async fn list_annotates_builtin_skill_ids_even_without_attachment() {
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
            ids.contains(&"example".to_string()),
            "the checked-in builtin example skill must appear even though nothing was attached: {ids:?}"
        );
    }
```

Leave every other test in this file's `mod tests` block unchanged.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::agent`
Expected: FAIL to compile — `list()`/`save()` still call `repo::skill::list_by_kind`, which no longer exists (removed in Task 2).

- [ ] **Step 3: Update `list()`**

Find this block inside `list()`:

```rust
    let builtin_ids: Vec<String> = repo::skill::list_by_kind(&state.db, "builtin")
        .await?
        .into_iter()
        .map(|s| s.id)
        .collect();
```

Replace with:

```rust
    let builtin_ids: Vec<String> = repo::skill::list_builtin().into_iter().map(|s| s.id).collect();
```

(No `.await?` — `list_builtin()` is now sync and infallible.)

- [ ] **Step 4: Update `save()`**

Find this block inside `save()`:

```rust
    let valid_custom_ids: std::collections::HashSet<String> =
        repo::skill::list_by_kind(&state.db, "custom")
            .await?
            .into_iter()
            .map(|s| s.id)
            .collect();
```

Replace with:

```rust
    let valid_custom_ids: std::collections::HashSet<String> = repo::skill::list(&state.db)
        .await?
        .into_iter()
        .map(|s| s.id)
        .collect();
```

(`repo::skill::list` now returns exactly the DB's custom skills — equivalent to the old `list_by_kind(pool, "custom")`.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::agent`
Expected: PASS (all tests in this module)

- [ ] **Step 6: Run the full Rust baseline**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings && cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: all green. This is the last Rust code task — a full green baseline here means every backend call site has been migrated off DB-seeded builtins.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/engine/commands/agent.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(commands): agentDef.list/save source builtin skill ids from the folder

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Bundle the `skills/` folder via Tauri resources

**Files:**
- Modify: `src-tauri/tauri.conf.json`

**Interfaces:**
- Consumes: `src-tauri/skills/` (Task 3).

- [ ] **Step 1: Edit the config**

In `src-tauri/tauri.conf.json`, inside the existing `"bundle": { ... }` object (which currently has `"active"`, `"targets"`, `"icon"`), add:

```json
    "resources": {
      "skills": "skills"
    }
```

The full `"bundle"` block should read:

```json
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "resources": {
      "skills": "skills"
    }
  }
```

The map form (`{"skills": "skills"}`, source path → target path, both relative to `tauri.conf.json`'s own directory) copies the whole `skills/` directory into `Contents/Resources/skills/` on macOS, preserving the directory name — a bare glob (`"resources": ["skills/*"]`) would instead flatten each skill's subdirectory directly into `Resources/`, losing the `skills/` prefix that `bundled_skills_dir()` (Task 3) expects.

- [ ] **Step 2: Validate the JSON**

Run: `python3 -c "import json; json.load(open('src-tauri/tauri.conf.json'))" && echo VALID_JSON`
Expected: `VALID_JSON` printed, no exception.

- [ ] **Step 3: Attempt a full build if your environment supports it**

Run: `pnpm tauri build --debug 2>&1 | tail -60` (or `cargo tauri build --debug` if the `tauri` pnpm script isn't wired — check `package.json`'s scripts first)

If this succeeds and produces a `.app` bundle: locate it (typically under `src-tauri/target/debug/bundle/macos/*.app`) and run `ls "<path-to-.app>/Contents/Resources/skills/example/"` to confirm `SKILL.md` is present inside the bundle.

If your environment cannot complete a full Tauri/macOS app build (no code-signing toolchain, no display, sandboxed CI, etc.), or the build fails for infrastructure reasons unrelated to this JSON change: **say so explicitly in your report — do not claim the bundle was verified if it wasn't.** This mirrors the rest of this session's established pattern for Tauri-runtime-dependent verification that an implementer subagent may not be able to perform.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tauri.conf.json
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(build): bundle the skills/ folder into the app via Tauri resources

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: Final verification (backend + confirm frontend untouched/unbroken)

**Files:** none (verification only).

- [ ] **Step 1: Full Rust baseline**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings && cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: all green.

- [ ] **Step 2: Full frontend baseline (confirm no frontend code needed changing)**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: both green, with ZERO frontend files modified across this entire plan (`git diff --stat <plan-start-sha>..HEAD -- src/` should be empty).

- [ ] **Step 3: Confirm no remaining references to the removed DB `kind` mechanism**

Run: `grep -rn "list_by_kind" src-tauri/src/ || echo "CLEAN — no references remain"`
Expected: `CLEAN — no references remain` (the function was fully removed in Task 2 and every caller migrated in Tasks 3-5).

- [ ] **Step 4: Walk the acceptance criteria**

- [ ] Migration 0005 applies cleanly on top of 0004 and drops `skill.kind` (Task 1's tests prove this).
- [ ] `repo::skill::list_builtin()` discovers the checked-in `skills/example/` fixture in a `cargo test` run (Task 3's test proves this).
- [ ] `commands::skill.list/save/delete` correctly source/reject builtins from the folder, not the DB (Task 4's tests).
- [ ] `commands::agent.list/save` annotate/validate against folder-sourced builtin ids (Task 5's tests).
- [ ] `tauri.conf.json` bundles `skills/` into `Contents/Resources/skills/` (Task 6 — full verification may be a disclosed manual/environment-limited gap, same as the original Skill System plan's Tauri-runtime gaps).
- [ ] Zero frontend files changed.

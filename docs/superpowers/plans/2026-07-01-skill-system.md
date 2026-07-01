# Skill System v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user attach reusable "skill" prompt modules (builtin, mandatory + custom, user-authored) to a `cli`-type `AgentDefinition`, injected via a sidecar file pointed to from the agent's bootstrap preamble at launch.

**Architecture:** New `repo::skill` module (mirrors `repo::agent_definition`) backs a new `skill.*` IPC namespace and a new `SkillLibrary`/`SkillEditor` UI pair (mirrors `Library`/`Builder`). `agent_skill` (already in schema) stores custom attachments only — builtin skills are fetched unconditionally, never stored as `agent_skill` rows, which is what makes "cannot be detached" structural rather than UI-only. At `cli` spawn, concatenated skill content is written to a per-instance sidecar file (NOT embedded in the preamble string, which must stay single-line/`=`-free for Codex — see ADR 0001); the preamble gets one sanitized sentence pointing at that file. `session.launched_skill_ids` snapshots what was actually used so the Roster can show a "Restart to apply" badge on drift.

**Tech Stack:** Tauri v2, Rust (sqlx + chain-builder, SQLite WAL), React 19 + TypeScript strict + Tailwind v4.

## Global Constraints

- Rust baseline: `cargo test --lib` + `cargo clippy --all-targets -- -D warnings` + `cargo fmt --check`, run via `--manifest-path src-tauri/Cargo.toml`.
- Frontend baseline: `pnpm exec tsc --noEmit` + `pnpm build` (>500kB chunk advisory OK).
- App UI copy is English; no fabricated data.
- Secrets never touch this feature (no env/API-key surface here) — not applicable, but keep in mind if a future skill references credentials.
- `bootstrap_preamble`'s return value MUST stay a single line with no `=` character (enforced by existing tests in `agentctx.rs`) — this is why skill content goes to a sidecar file, never into that string directly.
- `agent_skill.skill_id` already has `ON DELETE CASCADE` (`0001_init.sql:144`) — no extra cleanup code needed on skill delete.
- Only commit when a task's steps say to; each task's commit is a real `git` commit authored as `detoro <meanstack20@gmail.com>` (mirror this session's established authorship convention) with a `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer.

---

### Task 1: Migration `0004_skill_system.sql`

**Files:**
- Create: `src-tauri/src/engine/migrations/0004_skill_system.sql`
- Modify: `src-tauri/src/engine/db.rs:96-97` (append a `version < 4` gate)
- Test: `src-tauri/src/engine/db.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `skill.kind` (`TEXT NOT NULL CHECK(kind IN ('builtin','custom'))`), `skill.content` (`TEXT NOT NULL DEFAULT ''`), `session.launched_skill_ids` (`TEXT`, nullable) — every later task reads/writes these exact column names.

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block at the bottom of `src-tauri/src/engine/db.rs`:

```rust
    /// Migration 0004 adds `skill.kind`/`skill.content` and
    /// `session.launched_skill_ids`, and bumps user_version to 4.
    #[tokio::test]
    async fn migrate_adds_skill_system_columns() {
        let pool = connect_in_memory().await;

        // A plain INSERT into skill must now require `kind` (NOT NULL, no
        // default) to succeed only when kind is supplied; content defaults to ''.
        sqlx::query("INSERT INTO skill (id, name, kind) VALUES ('sk1', 'Test', 'builtin')")
            .execute(&pool)
            .await
            .expect("insert with kind should succeed");

        let (kind, content): (String, String) =
            sqlx::query_as("SELECT kind, content FROM skill WHERE id = 'sk1'")
                .fetch_one(&pool)
                .await
                .expect("select should succeed");
        assert_eq!(kind, "builtin");
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
        assert_eq!(version, 4);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml migrate_adds_skill_system_columns -- --nocapture`
Expected: FAIL — `no such column: kind` (or similar), since the migration file doesn't exist yet.

- [ ] **Step 3: Write the migration file**

```sql
-- Extend the dormant `skill` table (0001_init.sql) with a builtin/custom
-- discriminator (mirrors `tool.kind`) and the actual instructional content
-- previously missing — `description` stays a short UI blurb, `content` is
-- what gets injected into a launched cli agent's skill sidecar file.
ALTER TABLE skill ADD COLUMN kind TEXT NOT NULL DEFAULT 'custom' CHECK(kind IN ('builtin', 'custom'));
ALTER TABLE skill ADD COLUMN content TEXT NOT NULL DEFAULT '';

-- No builtin skill rows are seeded yet in v1 — the mechanism ships with zero
-- rows; product can add `INSERT OR IGNORE INTO skill (...) VALUES (...)` rows
-- in a later migration without needing to touch this one.

-- Snapshot of which skill ids were actually used at the last launch (JSON
-- array, ordered: builtin first, then custom by agent_skill.sort_order — see
-- repo::skill::content_for_agent). Compared against an agent definition's
-- CURRENT attachments to show a "Restart to apply" badge on drift.
ALTER TABLE session ADD COLUMN launched_skill_ids TEXT;
```

- [ ] **Step 4: Wire the migration into `migrate()`**

In `src-tauri/src/engine/db.rs`, after the `if version < 3 { ... }` block (ends around line 96):

```rust
    if version < 4 {
        sqlx::raw_sql(include_str!("migrations/0004_skill_system.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 4;")
            .execute(&mut *tx)
            .await?;
    }

```

(insert before the existing `tx.commit().await?;` line)

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml migrate_adds_skill_system_columns -- --nocapture`
Expected: PASS

- [ ] **Step 6: Run the full existing db test module to check for regressions**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::db::tests`
Expected: all PASS, including the pre-existing `migrate_is_idempotent` and `migrate_creates_all_tables` tests (unaffected — no new tables, only new columns).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/engine/migrations/0004_skill_system.sql src-tauri/src/engine/db.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(db): migration 0004 — skill.kind/content + session.launched_skill_ids

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: `repo::skill` — row struct + core CRUD

**Files:**
- Create: `src-tauri/src/engine/repo/skill.rs`
- Modify: `src-tauri/src/engine/repo/mod.rs` (add `pub mod skill;`)

**Interfaces:**
- Consumes: `super::cb_err` (from `repo/mod.rs`).
- Produces: `SkillRow { id, name, description: Option<String>, content: String, kind: String, icon: Option<String> }`; `list(pool)`, `list_by_kind(pool, kind: &str)`, `get(pool, id)`, `create(pool, name, description, content) -> SkillRow` (always `kind="custom"`), `update(pool, id, name, description, content) -> Option<SkillRow>`, `delete(pool, id) -> bool` — all consumed by Task 8's command handlers.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/engine/repo/skill.rs` with just the test module first:

```rust
//! Skill repository — reusable prompt modules attached to `AgentDefinition`s.
//! Mirrors `agent_definition.rs`'s pattern. Builtin-vs-custom AUTHORIZATION
//! (rejecting an edit/delete of a `kind = 'builtin'` row) is enforced at the
//! COMMAND layer (`commands::skill`), not here — this module is a plain CRUD
//! mirror, same division of responsibility as `agent_definition`'s
//! `permission_mode` allowlist check living in `commands::agent`, not the repo.

#[cfg(test)]
mod tests {
    use crate::engine::db::connect_in_memory;

    #[tokio::test]
    async fn create_then_get_roundtrip() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Reviewer", Some("Reviews diffs"), "Always check for X")
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
        assert!(super::get(&pool, &row.id).await.expect("get failed").is_none());
        assert!(!super::delete(&pool, &row.id).await.expect("second delete failed"));
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
        super::create(&pool, "Custom", None, "c").await.expect("create failed");

        let builtins = super::list_by_kind(&pool, "builtin").await.expect("list failed");
        assert_eq!(builtins.len(), 1);
        assert_eq!(builtins[0].id, "sk-b");

        let customs = super::list_by_kind(&pool, "custom").await.expect("list failed");
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: FAIL to compile — `super::create` etc. don't exist yet.

- [ ] **Step 3: Write the implementation (above the test module)**

```rust
use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite, Value as Bind};
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Decoded row from the `skill` table. `kind` is `"builtin"` or `"custom"`
/// (DB CHECK enforces this). `content` is always present (defaults to `""`
/// at the schema level, never NULL).
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
                description.map(|d| Bind::Text(d.to_owned())).unwrap_or(Bind::Null),
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
                description.map(|d| Bind::Text(d.to_owned())).unwrap_or(Bind::Null),
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
pub async fn delete(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    let res = sqlx::query("DELETE FROM skill WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}
```

- [ ] **Step 4: Register the module**

In `src-tauri/src/engine/repo/mod.rs`, add alongside the other `pub mod` lines (alphabetical order, between `provider` and `snapshot`... actually between `session` and `snapshot` alphabetically for `skill`):

```rust
pub mod session;
pub mod skill;
pub mod snapshot;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: PASS (7 tests)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/repo/skill.rs src-tauri/src/engine/repo/mod.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(repo): add skill repository — CRUD mirroring agent_definition

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: `repo::skill` — agent_skill attachment helpers

**Files:**
- Modify: `src-tauri/src/engine/repo/skill.rs`

**Interfaces:**
- Consumes: `SkillRow` (Task 2).
- Produces: `attached_to_agent(pool, agent_def_id) -> Vec<SkillRow>`, `set_custom_attachments(pool, agent_def_id, skill_ids: &[String])`, `custom_skill_ids_by_agent(pool) -> HashMap<String, Vec<String>>`, `attached_counts(pool) -> HashMap<String, i64>` — all consumed by Task 8/9's command handlers.

- [ ] **Step 1: Write the failing tests**

Add to `skill.rs`'s `mod tests` block (needs new fixtures — a real `agent_definition` to attach to):

```rust
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
        let s1 = super::create(&pool, "First", None, "1").await.expect("create failed");
        let s2 = super::create(&pool, "Second", None, "2").await.expect("create failed");

        // Attach in REVERSE order to prove sort_order (not insertion order) wins.
        super::set_custom_attachments(&pool, &def_id, &[s2.id.clone(), s1.id.clone()])
            .await
            .expect("set_custom_attachments failed");

        let attached = super::attached_to_agent(&pool, &def_id).await.expect("query failed");
        assert_eq!(attached.len(), 2);
        assert_eq!(attached[0].id, s2.id, "sort_order 0 must come first");
        assert_eq!(attached[1].id, s1.id);
    }

    #[tokio::test]
    async fn set_custom_attachments_replaces_not_appends() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "A", None, "a").await.expect("create failed");
        let s2 = super::create(&pool, "B", None, "b").await.expect("create failed");

        super::set_custom_attachments(&pool, &def_id, &[s1.id.clone()])
            .await
            .expect("first set failed");
        super::set_custom_attachments(&pool, &def_id, &[s2.id.clone()])
            .await
            .expect("second set failed");

        let attached = super::attached_to_agent(&pool, &def_id).await.expect("query failed");
        assert_eq!(attached.len(), 1, "second call must REPLACE, not append");
        assert_eq!(attached[0].id, s2.id);
    }

    #[tokio::test]
    async fn set_custom_attachments_empty_clears() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "A", None, "a").await.expect("create failed");
        super::set_custom_attachments(&pool, &def_id, &[s1.id]).await.expect("set failed");
        super::set_custom_attachments(&pool, &def_id, &[]).await.expect("clear failed");

        let attached = super::attached_to_agent(&pool, &def_id).await.expect("query failed");
        assert!(attached.is_empty());
    }

    #[tokio::test]
    async fn custom_skill_ids_by_agent_groups_correctly() {
        let pool = connect_in_memory().await;
        let def1 = fixture_agent_def(&pool).await;
        let def2 = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "A", None, "a").await.expect("create failed");
        let s2 = super::create(&pool, "B", None, "b").await.expect("create failed");
        super::set_custom_attachments(&pool, &def1, &[s1.id.clone(), s2.id.clone()])
            .await
            .expect("set failed");
        super::set_custom_attachments(&pool, &def2, &[s1.id.clone()])
            .await
            .expect("set failed");

        let map = super::custom_skill_ids_by_agent(&pool).await.expect("query failed");
        assert_eq!(map.get(&def1).cloned().unwrap_or_default().len(), 2);
        assert_eq!(map.get(&def2).cloned().unwrap_or_default(), vec![s1.id]);
    }

    #[tokio::test]
    async fn attached_counts_counts_across_agents() {
        let pool = connect_in_memory().await;
        let def1 = fixture_agent_def(&pool).await;
        let def2 = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "Shared", None, "s").await.expect("create failed");
        super::set_custom_attachments(&pool, &def1, &[s1.id.clone()]).await.expect("set failed");
        super::set_custom_attachments(&pool, &def2, &[s1.id.clone()]).await.expect("set failed");

        let counts = super::attached_counts(&pool).await.expect("query failed");
        assert_eq!(counts.get(&s1.id).copied(), Some(2));
    }

    /// Deleting a skill cascades its `agent_skill` rows without touching the
    /// agent_definition (locks in the ON DELETE CASCADE relied on in the ADR).
    #[tokio::test]
    async fn delete_skill_cascades_agent_skill_rows() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "Doomed", None, "d").await.expect("create failed");
        super::set_custom_attachments(&pool, &def_id, &[s1.id.clone()])
            .await
            .expect("set failed");

        super::delete(&pool, &s1.id).await.expect("delete failed");

        let attached = super::attached_to_agent(&pool, &def_id).await.expect("query failed");
        assert!(attached.is_empty(), "agent_skill row must cascade away");
        assert!(
            crate::engine::repo::agent_definition::exists(&pool, &def_id)
                .await
                .expect("exists check failed"),
            "agent_definition itself must be untouched"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: FAIL to compile — the new functions don't exist yet.

- [ ] **Step 3: Write the implementation**

Add to `skill.rs`, above the `mod tests` block:

```rust
use std::collections::HashMap;

/// All skills attached to `agent_def_id` via `agent_skill`, ordered by
/// `sort_order` then `id` as a stable tie-breaker. Builtin skills are NEVER
/// stored here — see `content_for_agent`, which fetches them separately and
/// unconditionally.
pub async fn attached_to_agent(pool: &SqlitePool, agent_def_id: &str) -> sqlx::Result<Vec<SkillRow>> {
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
        sqlx::query("INSERT INTO agent_skill (agent_def_id, skill_id, sort_order) VALUES (?, ?, ?)")
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
pub async fn custom_skill_ids_by_agent(pool: &SqlitePool) -> sqlx::Result<HashMap<String, Vec<String>>> {
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
pub async fn attached_counts(pool: &SqlitePool) -> sqlx::Result<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT skill_id, COUNT(*) FROM agent_skill GROUP BY skill_id")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: PASS (13 tests total)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/engine/repo/skill.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(repo): skill attachment helpers (agent_skill join, replace, counts)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: `repo::skill` — `content_for_agent` (sidecar body builder)

**Files:**
- Modify: `src-tauri/src/engine/repo/skill.rs`

**Interfaces:**
- Consumes: `list_by_kind` (Task 2), `attached_to_agent` (Task 3).
- Produces: `content_for_agent(pool, agent_def_id) -> (String, Vec<String>)` — the `(sidecar_body, ordered_skill_ids_used)` pair consumed by Task 10 (`commands::instance::spawn`).

- [ ] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn content_for_agent_orders_builtin_then_custom_with_headers() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        sqlx::query("INSERT INTO skill (id, name, content, kind) VALUES ('sk-b', 'Base', 'Be careful', 'builtin')")
            .execute(&pool)
            .await
            .expect("seed builtin failed");
        let custom = super::create(&pool, "Extra", None, "Do X").await.expect("create failed");
        super::set_custom_attachments(&pool, &def_id, &[custom.id.clone()])
            .await
            .expect("set failed");

        let (body, ids) = super::content_for_agent(&pool, &def_id).await.expect("query failed");

        assert_eq!(ids, vec!["sk-b".to_string(), custom.id.clone()], "builtin must come first");
        let base_pos = body.find("## Skill: Base").expect("Base header missing");
        let extra_pos = body.find("## Skill: Extra").expect("Extra header missing");
        assert!(base_pos < extra_pos, "builtin section must precede custom section");
        assert!(body.contains("Be careful"));
        assert!(body.contains("Do X"));
    }

    #[tokio::test]
    async fn content_for_agent_empty_when_nothing_attached() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let (body, ids) = super::content_for_agent(&pool, &def_id).await.expect("query failed");
        assert_eq!(body, "");
        assert!(ids.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: FAIL to compile — `content_for_agent` doesn't exist yet.

- [ ] **Step 3: Write the implementation**

```rust
/// Build the concatenated skill body for one agent definition's `cli` launch,
/// plus the ordered list of skill ids used (for `session.launched_skill_ids`).
/// Builtin skills come first (fixed `id` order, via `list_by_kind`), then
/// custom skills by `sort_order` (via `attached_to_agent`). Each skill renders
/// as a `## Skill: {name}` header followed by its `content`, sections
/// separated by a blank line. Returns `("", [])` when nothing is attached and
/// no builtin skills exist — the caller (`commands::instance::spawn`) treats
/// an empty body as "skip the sidecar file entirely".
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill`
Expected: PASS (15 tests total)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/engine/repo/skill.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(repo): skill::content_for_agent builds the sidecar body

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: `repo::session` — `launched_skill_ids` snapshot

**Files:**
- Modify: `src-tauri/src/engine/repo/session.rs`

**Interfaces:**
- Produces: `SessionRow.launched_skill_ids: Option<String>` (serializes as `launchedSkillIds?: string[]`), `set_launched_skill_ids(pool, session_id, skill_ids: &[String])` — consumed by Task 10.

- [ ] **Step 1: Write the failing test**

Add to `session.rs`'s `mod tests` block:

```rust
    /// set_launched_skill_ids persists an ordered JSON array, readable back
    /// via get(); a fresh session (create_for_instance) starts with None.
    #[tokio::test]
    async fn set_launched_skill_ids_roundtrips() {
        let pool = connect_in_memory().await;
        let wa_id = fixture_instance(&pool).await;
        let row = create_for_instance(&pool, &wa_id)
            .await
            .expect("create_for_instance failed");
        assert!(row.launched_skill_ids.is_none(), "fresh session has no launch snapshot yet");

        set_launched_skill_ids(&pool, &row.id, &["sk-1".to_string(), "sk-2".to_string()])
            .await
            .expect("set_launched_skill_ids failed");

        let fetched = get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("session exists");
        let json = serde_json::to_value(&fetched).expect("serialize failed");
        assert_eq!(
            json.get("launchedSkillIds").and_then(|v| v.as_array()).map(|a| a.len()),
            Some(2),
            "launchedSkillIds must serialize as a JSON array"
        );
    }

    /// Calling set_launched_skill_ids with an empty slice stores `[]`, not
    /// NULL — distinguishes "launched with zero skills" from "never launched".
    #[tokio::test]
    async fn set_launched_skill_ids_empty_slice_stores_empty_array_not_null() {
        let pool = connect_in_memory().await;
        let wa_id = fixture_instance(&pool).await;
        let row = create_for_instance(&pool, &wa_id).await.expect("create failed");

        set_launched_skill_ids(&pool, &row.id, &[]).await.expect("set failed");

        let fetched = get(&pool, &row.id).await.expect("get failed").expect("exists");
        assert_eq!(fetched.launched_skill_ids.as_deref(), Some("[]"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::session`
Expected: FAIL to compile — `launched_skill_ids` field / `set_launched_skill_ids` fn don't exist.

- [ ] **Step 3: Write the implementation**

In `session.rs`, add the JSON-text serializer helper (near the top, after imports):

```rust
/// Serialize the `launched_skill_ids` JSON-array-text column into a
/// structured JSON array. Mirrors `agent_definition::serialize_json_text`
/// (kept local rather than shared — same trivial helper, different module).
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
```

Update `SessionRow`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    pub id: String,
    pub workspace_agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<i64>,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<String>,
    /// JSON array of skill ids used at the last launch — see
    /// `repo::skill::content_for_agent`. `None` until the first launch.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "serialize_json_text")]
    pub launched_skill_ids: Option<String>,
}
```

Update `COLS`:

```rust
const COLS: [&str; 7] = [
    "id",
    "workspace_agent_id",
    "context_tokens",
    "context_limit",
    "started_at",
    "last_active_at",
    "launched_skill_ids",
];
```

Update `create_for_instance`'s INSERT and returned struct (add `launched_skill_ids`):

```rust
    QueryBuilder::<Sqlite>::table("session")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("workspace_agent_id", Bind::Text(workspace_agent_id.clone())),
            ("context_tokens", Bind::I64(0)),
            ("context_limit", Bind::I64(DEFAULT_CONTEXT_LIMIT)),
            ("started_at", Bind::Text(started_at.clone())),
            ("last_active_at", Bind::Null),
            ("launched_skill_ids", Bind::Null),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(SessionRow {
        id,
        workspace_agent_id,
        context_tokens: Some(0),
        context_limit: Some(DEFAULT_CONTEXT_LIMIT),
        started_at,
        last_active_at: None,
        launched_skill_ids: None,
    })
```

Add the setter (near `set_context_tokens`):

```rust
/// Persist the ordered list of skill ids actually used for the most recent
/// launch. Compared against an agent definition's CURRENT attachments
/// (`repo::skill::custom_skill_ids_by_agent` + builtin ids) to detect drift
/// and show a "Restart to apply" badge in the Roster.
pub async fn set_launched_skill_ids(
    pool: &SqlitePool,
    session_id: &str,
    skill_ids: &[String],
) -> sqlx::Result<()> {
    let json = serde_json::to_string(skill_ids).expect("serializing Vec<String> is infallible");
    QueryBuilder::<Sqlite>::table("session")
        .update([("launched_skill_ids", Bind::Text(json))])
        .where_eq("id", session_id)
        .execute(pool)
        .await
        .map_err(cb_err)?;
    Ok(())
}
```

- [ ] **Step 4: Fix the existing `camel_case_contract` test's negative assertions**

The existing test in `session.rs` doesn't assert anything about `launched_skill_ids`, so it needs no change — but double check by running it (next step) since adding a new `skip_serializing_if` field to a `PartialEq` struct doesn't break existing equality assertions (all pre-existing tests construct rows via `create_for_instance`, which now sets `launched_skill_ids: None` consistently on both sides of every `assert_eq!`).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::session`
Expected: PASS (all existing + 2 new tests)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/repo/session.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(repo): session.launched_skill_ids launch-snapshot column

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: `repo::workspace_agent` — join `launched_skill_ids` for the Roster

**Files:**
- Modify: `src-tauri/src/engine/repo/workspace_agent.rs`

**Interfaces:**
- Produces: `WorkspaceAgentWithSkills { id, workspace_id, agent_def_id, status, added_at, launched_skill_ids: Option<String> }`, `list_by_workspace_with_launched_skills(pool, workspace_id) -> Vec<WorkspaceAgentWithSkills>` — consumed by Task 10 (`commands::instance::list`).

- [ ] **Step 1: Write the failing test**

Add to `workspace_agent.rs`'s `mod tests` block:

```rust
    /// list_by_workspace_with_launched_skills LEFT JOINs the paired session's
    /// launched_skill_ids — present after a launch snapshot, NULL before one.
    #[tokio::test]
    async fn list_by_workspace_with_launched_skills_joins_session() {
        let pool = connect_in_memory().await;
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
        let inst = instantiate(&pool, &ws.id, &def.id).await.expect("instantiate failed");
        let session = crate::engine::repo::session::get_by_instance(&pool, &inst.id)
            .await
            .expect("get session failed")
            .expect("session exists");

        // Before any launch snapshot: NULL.
        let before = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(before.len(), 1);
        assert!(before[0].launched_skill_ids.is_none());

        crate::engine::repo::session::set_launched_skill_ids(&pool, &session.id, &["sk-1".to_string()])
            .await
            .expect("set failed");

        let after = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(after[0].launched_skill_ids.as_deref(), Some(r#"["sk-1"]"#));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::workspace_agent`
Expected: FAIL to compile — `list_by_workspace_with_launched_skills` doesn't exist yet.

- [ ] **Step 3: Write the implementation**

Add after `list_by_workspace` in `workspace_agent.rs`:

```rust
/// Like [`WorkspaceAgentRow`] but annotated with the paired session's
/// `launched_skill_ids` (raw JSON-array text, `None` before any launch). Used
/// ONLY by `commands::instance::list` so the Roster can detect skill drift
/// without a second IPC round-trip per instance.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAgentWithSkills {
    pub id: String,
    pub workspace_id: String,
    pub agent_def_id: String,
    pub status: String,
    pub added_at: String,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "serialize_launched_ids")]
    pub launched_skill_ids: Option<String>,
}

/// Same shape as `session::serialize_json_text` — kept local (small, trivial,
/// different module) rather than shared.
fn serialize_launched_ids<S>(opt: &Option<String>, ser: S) -> Result<S::Ok, S::Error>
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

/// Return all workspace_agents for a workspace (same ordering as
/// `list_by_workspace`) LEFT JOINed with their session's `launched_skill_ids`.
/// A workspace_agent with no session yet (should not normally happen —
/// `instantiate` creates both atomically) yields `None`, same as one whose
/// session simply hasn't launched.
pub async fn list_by_workspace_with_launched_skills(
    pool: &SqlitePool,
    workspace_id: &str,
) -> sqlx::Result<Vec<WorkspaceAgentWithSkills>> {
    sqlx::query_as::<_, WorkspaceAgentWithSkills>(
        "SELECT wa.id, wa.workspace_id, wa.agent_def_id, wa.status, wa.added_at, \
         sess.launched_skill_ids \
         FROM workspace_agent wa \
         LEFT JOIN session sess ON sess.workspace_agent_id = wa.id \
         WHERE wa.workspace_id = ? \
         ORDER BY wa.added_at ASC, wa.id ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::workspace_agent`
Expected: PASS (all existing + 1 new test)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/engine/repo/workspace_agent.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(repo): join launched_skill_ids into workspace_agent listing

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 7: `agentctx` — skill sidecar file + pointer sentence

**Files:**
- Modify: `src-tauri/src/engine/agentctx.rs`

**Interfaces:**
- Consumes: `sanitize_field` (existing, same file).
- Produces: `write_skill_sidecar(instance_id: &str, body: &str) -> std::io::Result<PathBuf>`, `skill_pointer_sentence(path: &Path) -> String` — consumed by Task 10.

- [ ] **Step 1: Write the failing tests**

Add to `agentctx.rs`'s `mod tests` block:

```rust
    #[test]
    fn skill_pointer_sentence_is_single_line_and_equals_free() {
        let s = super::skill_pointer_sentence(std::path::Path::new("/tmp/a=b\nc.md"));
        assert!(!s.contains('\n'), "no newline: {s}");
        assert!(!s.contains('='), "no '=': {s}");
    }

    #[test]
    fn skill_pointer_sentence_names_the_path() {
        let s = super::skill_pointer_sentence(std::path::Path::new("/tmp/inst-a.md"));
        assert!(s.contains("/tmp/inst-a.md"), "{s}");
    }

    /// The invariant the whole feature exists to protect: appending the skill
    /// pointer sentence to a real preamble must NOT reintroduce a newline or
    /// '=', even when the underlying skill body (never embedded here) is
    /// pathological — see ADR 0001.
    #[test]
    fn preamble_with_skill_pointer_appended_stays_single_line_and_equals_free() {
        let p = bootstrap_preamble("Atlas", Some("builder"), "My Repo", "ws_123", "inst_a");
        let pointer = super::skill_pointer_sentence(std::path::Path::new("/tmp/inst_a.md"));
        let combined = format!("{p} {pointer}");
        assert!(!combined.contains('\n'), "no newline: {combined}");
        assert!(!combined.contains('='), "no '=': {combined}");
    }

    #[test]
    fn write_skill_sidecar_writes_and_returns_path() {
        let body = "## Skill: Test\n\nkey=value works fine in a real FILE";
        let path = super::write_skill_sidecar("test-instance-xyz", body)
            .expect("write_skill_sidecar failed");
        let contents = std::fs::read_to_string(&path).expect("read back failed");
        assert_eq!(contents, body);
        let _ = std::fs::remove_file(&path); // test cleanup
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::agentctx`
Expected: FAIL to compile — `skill_pointer_sentence` / `write_skill_sidecar` don't exist.

- [ ] **Step 3: Write the implementation**

Add to `agentctx.rs`, after `ensure_conclave_shim`'s two `#[cfg]` variants:

```rust
/// Write concatenated skill content for one instance to a sidecar file under
/// the Conclave data dir, overwriting on each launch. The content itself
/// (real markdown — may contain '\n' and '=') NEVER enters
/// `bootstrap_preamble`'s return value directly (that string must stay a
/// single line with no '=', see its own doc comment); only a pointer sentence
/// to this file does. Owner-only (`0700`) dir, mirroring
/// `ensure_conclave_shim`'s `bin` dir.
#[cfg(unix)]
pub fn write_skill_sidecar(instance_id: &str, body: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;

    let dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no user data directory"))?
        .join("Conclave")
        .join("skills");
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;

    let path = dir.join(format!("{instance_id}.md"));
    std::fs::write(&path, body)?;
    Ok(path)
}

#[cfg(not(unix))]
pub fn write_skill_sidecar(_instance_id: &str, _body: &str) -> std::io::Result<PathBuf> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "skill sidecar files are only supported on unix",
    ))
}

/// One sanitized, single-line sentence pointing a CLI agent at its skill
/// sidecar file — the ONLY thing appended to `bootstrap_preamble`'s return
/// value on top of skill content. Runs the same `sanitize_field` the rest of
/// the preamble uses, so a pathological path can't reintroduce a newline or
/// '=' (defense in depth — a real filesystem path shouldn't contain either).
pub fn skill_pointer_sentence(path: &std::path::Path) -> String {
    let path = sanitize_field(&path.display().to_string());
    format!(
        "Additional standing instructions for this session are at {path} — read that file before your first response."
    )
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::agentctx`
Expected: PASS (all existing + 4 new tests)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/engine/agentctx.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(agentctx): skill sidecar file + single-line preamble pointer

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 8: `commands::skill` — IPC handlers + router wiring

**Files:**
- Create: `src-tauri/src/engine/commands/skill.rs`
- Modify: `src-tauri/src/engine/commands/mod.rs` (add `pub mod skill;`)
- Modify: `src-tauri/src/engine/router.rs` (add `skill.*` dispatch)

**Interfaces:**
- Consumes: `repo::skill::{list, get, create, update, delete, attached_counts}` (Tasks 2-3).
- Produces: `skill.list` / `skill.save` / `skill.delete` IPC commands — consumed by Task 12 (frontend `ipc.skill.*`).

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/engine/commands/skill.rs` with implementation AND tests together. Command-layer tests use the existing `AppState::for_tests()` test-only constructor (`src-tauri/src/engine/state.rs:138` — an in-memory DB pool, migration applied, no real `AppHandle`). Do NOT hand-roll an `AppState { db, runtime }` struct literal — `AppState`'s `app`/`compact_pending` fields are private to `state.rs`, so a literal construction from another module will not compile.

```rust
//! Skill command handlers — CRUD for reusable prompt modules attached to
//! `AgentDefinition`s. Builtin-vs-custom AUTHORIZATION (rejecting a mutation
//! of a `kind = "builtin"` row) is enforced HERE, not in `repo::skill` (which
//! stays a plain CRUD mirror of `agent_definition`'s pattern) — same division
//! of responsibility as the `permission_mode` allowlist check living in
//! `commands::agent`, not the repo layer.

use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSkillReq {
    id: Option<String>,
    name: String,
    description: Option<String>,
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteSkillReq {
    id: String,
}

/// Return every skill (builtin first), each annotated with `attachedTo`: how
/// many agent definitions currently have it attached.
///
/// Maps to `skill.list` on the IPC bus.
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let skills = repo::skill::list(&state.db).await?;
    let counts = repo::skill::attached_counts(&state.db).await?;

    let mut value = serde_json::to_value(&skills).map_err(|e| AppError::Internal(e.to_string()))?;
    if let Some(arr) = value.as_array_mut() {
        for (item, skill) in arr.iter_mut().zip(skills.iter()) {
            item["attachedTo"] = serde_json::json!(counts.get(&skill.id).copied().unwrap_or(0));
        }
    }
    Ok(value)
}

/// Create or update a CUSTOM skill. Maps to `skill.save` on the IPC bus.
/// Rejects (`AppError::Invalid`) an attempt to edit a builtin skill.
pub async fn save(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SaveSkillReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if let Some(id) = req.id.as_deref() {
        let existing = repo::skill::get(&state.db, id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("skill id={id} not found")))?;
        if existing.kind == "builtin" {
            return Err(AppError::Invalid("cannot edit a builtin skill".into()));
        }
        let row = repo::skill::update(&state.db, id, &req.name, req.description.as_deref(), &req.content)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("skill id={id} not found")))?;
        return serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()));
    }

    let row = repo::skill::create(&state.db, &req.name, req.description.as_deref(), &req.content).await?;
    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Delete a CUSTOM skill. Maps to `skill.delete` on the IPC bus. Rejects
/// (`AppError::Invalid`) an attempt to delete a builtin skill. `agent_skill`
/// rows referencing it cascade away (`ON DELETE CASCADE`).
pub async fn delete(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DeleteSkillReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let existing = repo::skill::get(&state.db, &req.id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("skill id={} not found", req.id)))?;
    if existing.kind == "builtin" {
        return Err(AppError::Invalid("cannot delete a builtin skill".into()));
    }

    repo::skill::delete(&state.db, &req.id).await?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_creates_then_updates_custom_skill() {
        let state = AppState::for_tests().await;
        let created = save(
            &state,
            serde_json::json!({ "name": "Reviewer", "description": "desc", "content": "text" }),
        )
        .await
        .expect("create failed");
        let id = created["id"].as_str().expect("id present").to_owned();

        let updated = save(
            &state,
            serde_json::json!({ "id": id, "name": "Reviewer2", "content": "new text" }),
        )
        .await
        .expect("update failed");
        assert_eq!(updated["name"], "Reviewer2");
        assert_eq!(updated["content"], "new text");
    }

    #[tokio::test]
    async fn save_rejects_editing_builtin() {
        let state = AppState::for_tests().await;
        sqlx::query("INSERT INTO skill (id, name, kind) VALUES ('sk-b', 'Core', 'builtin')")
            .execute(&state.db)
            .await
            .expect("seed failed");

        let result = save(
            &state,
            serde_json::json!({ "id": "sk-b", "name": "Hacked", "content": "x" }),
        )
        .await;
        assert!(matches!(result, Err(AppError::Invalid(_))));
    }

    #[tokio::test]
    async fn delete_rejects_builtin_but_allows_custom() {
        let state = AppState::for_tests().await;
        sqlx::query("INSERT INTO skill (id, name, kind) VALUES ('sk-b', 'Core', 'builtin')")
            .execute(&state.db)
            .await
            .expect("seed failed");
        let created = save(&state, serde_json::json!({ "name": "Custom", "content": "c" }))
            .await
            .expect("create failed");
        let custom_id = created["id"].as_str().unwrap().to_owned();

        let builtin_delete = delete(&state, serde_json::json!({ "id": "sk-b" })).await;
        assert!(matches!(builtin_delete, Err(AppError::Invalid(_))));

        let custom_delete = delete(&state, serde_json::json!({ "id": custom_id })).await;
        assert!(custom_delete.is_ok());
    }

    #[tokio::test]
    async fn list_annotates_attached_to_count() {
        let state = AppState::for_tests().await;
        let created = save(&state, serde_json::json!({ "name": "S", "content": "c" }))
            .await
            .expect("create failed");
        let skill_id = created["id"].as_str().unwrap().to_owned();

        let def = repo::agent_definition::create(
            &state.db,
            &repo::agent_definition::AgentDefinitionInput {
                name: "A".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        repo::skill::set_custom_attachments(&state.db, &def.id, &[skill_id.clone()])
            .await
            .expect("attach failed");

        let listed = list(&state, Value::Null).await.expect("list failed");
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == skill_id)
            .expect("skill present");
        assert_eq!(item["attachedTo"], 1);
    }
}
```

Note: before writing the test module, open `src-tauri/src/engine/commands/agent.rs`'s own `#[cfg(test)] mod tests` (or another `commands::*` module's tests) to confirm the exact `AppState`/`Runtime` construction used elsewhere in this codebase, and match it exactly — the snippet above is the expected shape based on `AppState { db, runtime }`, but copy the REAL helper if the existing one differs (e.g. a different `Runtime` constructor name).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::skill`
Expected: FAIL to compile (module not registered yet) or FAIL to resolve `AppState`/`Runtime` construction — fix the test helper to match the real types before proceeding, then re-run.

- [ ] **Step 3: Register the module and router dispatch**

In `src-tauri/src/engine/commands/mod.rs`:

```rust
pub mod provider;
pub mod skill;
pub mod snapshot;
```

In `src-tauri/src/engine/router.rs`, add to the `use` list:

```rust
use crate::engine::commands::{
    agent, blackboard, cli, fusion, instance, message, provider, skill, snapshot, tool, workspace,
};
```

and add a new dispatch block (after the `agentDef` block, before `instance`):

```rust
        // ── skill ─────────────────────────────────────────────────────────
        "skill.list" => skill::list(state, payload).await,
        "skill.save" => skill::save(state, payload).await,
        "skill.delete" => skill::delete(state, payload).await,

```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::skill`
Expected: PASS (4 tests)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/engine/commands/skill.rs src-tauri/src/engine/commands/mod.rs src-tauri/src/engine/router.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(commands): skill.list/save/delete IPC handlers + router wiring

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 9: `commands::agent` — wire skill attachment into `save`/`list`

**Files:**
- Modify: `src-tauri/src/engine/commands/agent.rs`

**Interfaces:**
- Consumes: `repo::skill::{list_by_kind, set_custom_attachments, custom_skill_ids_by_agent}` (Tasks 2-3).
- Produces: `AgentDefinition.skillIds` populated on `agentDef.list` responses (list-only annotation, same precedent as `inWorkspaces`); `agentDef.save`'s `skillIds` field (already accepted, previously dead) now persists real `agent_skill` rows.

- [ ] **Step 1: Write the failing tests**

`src-tauri/src/engine/commands/agent.rs` currently has NO `#[cfg(test)] mod tests` block at all (confirmed — this task adds the first one). Append this whole block, including the `mod tests` wrapper, at the end of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn save_persists_and_replaces_skill_attachments() {
        let state = AppState::for_tests().await;
        let skill = repo::skill::create(&state.db, "S1", None, "c").await.expect("create skill failed");

        let created = save(
            &state,
            serde_json::json!({
                "name": "Atlas", "type": "cli", "harnessMode": "own",
                "skillIds": [skill.id],
            }),
        )
        .await
        .expect("create agent failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let attached = repo::skill::attached_to_agent(&state.db, &id).await.expect("query failed");
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].id, skill.id);

        // A second save with an EMPTY skillIds list must clear the attachment
        // (replace semantics, not merge).
        save(
            &state,
            serde_json::json!({
                "id": id, "name": "Atlas", "type": "cli", "harnessMode": "own",
                "skillIds": [],
            }),
        )
        .await
        .expect("update agent failed");
        let attached_after = repo::skill::attached_to_agent(&state.db, &id).await.expect("query failed");
        assert!(attached_after.is_empty());
    }

    #[tokio::test]
    async fn save_silently_drops_unknown_or_builtin_skill_ids() {
        let state = AppState::for_tests().await;
        sqlx::query("INSERT INTO skill (id, name, kind) VALUES ('sk-b', 'Core', 'builtin')")
            .execute(&state.db)
            .await
            .expect("seed failed");

        let created = save(
            &state,
            serde_json::json!({
                "name": "Atlas", "type": "cli", "harnessMode": "own",
                "skillIds": ["sk-b", "no-such-id"],
            }),
        )
        .await
        .expect("create failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let attached = repo::skill::attached_to_agent(&state.db, &id).await.expect("query failed");
        assert!(attached.is_empty(), "builtin/unknown ids must never become agent_skill rows");
    }

    #[tokio::test]
    async fn list_annotates_skill_ids() {
        let state = AppState::for_tests().await;
        let skill = repo::skill::create(&state.db, "S1", None, "c").await.expect("create failed");
        let created = save(
            &state,
            serde_json::json!({
                "name": "Atlas", "type": "cli", "harnessMode": "own",
                "skillIds": [skill.id],
            }),
        )
        .await
        .expect("create failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let listed = list(&state, Value::Null).await.expect("list failed");
        let item = listed.as_array().unwrap().iter().find(|d| d["id"] == id).unwrap();
        assert_eq!(item["skillIds"].as_array().map(|a| a.len()), Some(1));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::agent`
Expected: FAIL — `skillIds` sent to `save` has no effect yet (attachments stay empty); `list`'s response has no `skillIds` key.

- [ ] **Step 3: Wire `list()`**

Replace the body of `pub async fn list` in `agent.rs`:

```rust
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let items = repo::agent_definition::list_with_counts(&state.db).await?;
    let skill_map = repo::skill::custom_skill_ids_by_agent(&state.db).await?;

    let mut value = serde_json::to_value(&items).map_err(|e| AppError::Internal(e.to_string()))?;
    if let Some(arr) = value.as_array_mut() {
        for item in arr.iter_mut() {
            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
            if let Some(ids) = skill_map.get(&id) {
                item["skillIds"] = serde_json::json!(ids);
            }
        }
    }
    Ok(value)
}
```

- [ ] **Step 4: Wire `save()`**

Remove the `#[allow(dead_code)]` attribute above `skill_ids: Option<Vec<String>>` in `SaveAgentReq`, and update the doc comment above the struct from `/// `toolIds` / `skillIds` are accepted and forwarded without error but deferred to M5.` to `/// `toolIds` is accepted and forwarded without error but deferred (agent_tool wiring is out of scope for the v1 skill system). `skillIds` IS persisted — see the `set_custom_attachments` call in `save()`.`.

In `save()`, add this block right after the existing Keychain secret-writing loop (`for name in &old_secret_names { ... }`), before the final `serde_json::to_value(row)`:

```rust
    // Persist custom skill attachments (replace semantics). Filter to known
    // CUSTOM skill ids so a stale/tampered request can't create an
    // `agent_skill` row for a builtin skill (which is never attached via that
    // table — see `repo::skill::content_for_agent`) or a nonexistent id.
    let valid_custom_ids: std::collections::HashSet<String> =
        repo::skill::list_by_kind(&state.db, "custom")
            .await?
            .into_iter()
            .map(|s| s.id)
            .collect();
    let filtered_skill_ids: Vec<String> = req
        .skill_ids
        .unwrap_or_default()
        .into_iter()
        .filter(|id| valid_custom_ids.contains(id))
        .collect();
    repo::skill::set_custom_attachments(&state.db, &row.id, &filtered_skill_ids).await?;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::agent`
Expected: PASS (all existing + 3 new tests)

- [ ] **Step 6: Run full Rust baseline**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings && cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/engine/commands/agent.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(commands): agentDef.save persists skillIds, agentDef.list annotates them

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 10: `commands::instance` — inject skills at `cli` spawn, join skills into `list`

**Files:**
- Modify: `src-tauri/src/engine/commands/instance.rs`

**Interfaces:**
- Consumes: `repo::skill::content_for_agent` (Task 4), `agentctx::{write_skill_sidecar, skill_pointer_sentence}` (Task 7), `repo::session::set_launched_skill_ids` (Task 5), `repo::workspace_agent::list_by_workspace_with_launched_skills` (Task 6).
- Produces: `apply_skills_to_preamble(state, agent_def_id, instance_id, session_id, preamble) -> Result<String, AppError>` (new private helper); launched `cli` instances now read skill content via a sidecar file; `instance.list` response carries `launchedSkillIds`.

**Important — do not spawn a real process in tests.** `instance.rs`'s existing test module (`mod tests` at line 630) deliberately avoids exercising the `cli` dispatch branch of `spawn()` for anything beyond the "unconfigured cli_kind" error path — see `fixture_instance`'s own doc comment: *"cli would take the PTY path and try to spawn `claude`"*. Calling `spawn()` on a fully-configured `cli` agent in a test would really fork/exec a login shell. To keep this testable, extract the skill-injection logic out of `spawn()`'s `cli` branch into a small standalone `async fn` and unit-test THAT directly — never call `spawn()` with `cli_kind: Some(...)` in a test.

- [ ] **Step 1: Write the failing tests**

Add to `instance.rs`'s existing `#[cfg(test)] mod tests` block (it already has `use super::*;` and the `workspace`, `agent_definition`, `workspace_agent` repo imports — reuse those, do not re-import):

```rust
    #[tokio::test]
    async fn apply_skills_to_preamble_writes_sidecar_and_snapshot_when_attached() {
        let state = AppState::for_tests().await;
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: "Atlas".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        let skill = repo::skill::create(&state.db, "Reviewer", None, "Always check X")
            .await
            .expect("create skill failed");
        let inst_id = workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id;
        repo::skill::set_custom_attachments(&state.db, &def.id, &[skill.id.clone()])
            .await
            .expect("attach failed");
        let session = repo::session::get_by_instance(&state.db, &inst_id)
            .await
            .expect("get session failed")
            .expect("session exists");

        let result = apply_skills_to_preamble(&state, &def.id, &inst_id, &session.id, "BASE PREAMBLE".to_string())
            .await
            .expect("apply_skills_to_preamble failed");

        assert!(result.starts_with("BASE PREAMBLE "), "must extend, not replace, the base preamble: {result}");
        assert!(!result.contains('\n'), "no newline: {result}");
        assert!(!result.contains('='), "no '=': {result}");

        let updated_session = repo::session::get(&state.db, &session.id)
            .await
            .expect("get failed")
            .expect("session exists");
        assert_eq!(
            updated_session.launched_skill_ids.as_deref(),
            Some(format!("[\"{}\"]", skill.id).as_str())
        );
    }

    #[tokio::test]
    async fn apply_skills_to_preamble_is_noop_when_nothing_attached() {
        let state = AppState::for_tests().await;
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: "A".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        let inst_id = workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id;
        let session = repo::session::get_by_instance(&state.db, &inst_id)
            .await
            .expect("get failed")
            .expect("exists");

        let result = apply_skills_to_preamble(&state, &def.id, &inst_id, &session.id, "BASE".to_string())
            .await
            .expect("apply_skills_to_preamble failed");
        assert_eq!(result, "BASE", "no skills attached -> preamble passes through unchanged");

        let updated_session = repo::session::get(&state.db, &session.id)
            .await
            .expect("get failed")
            .expect("exists");
        assert_eq!(
            updated_session.launched_skill_ids.as_deref(),
            Some("[]"),
            "must snapshot an EMPTY array (launched-with-zero-skills), not leave it NULL"
        );
    }

    #[tokio::test]
    async fn list_annotates_launched_skill_ids() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await; // orchestrator type — safe, no cli/chat dispatch
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get failed")
            .expect("exists");
        repo::session::set_launched_skill_ids(&state.db, &session.id, &["sk-x".to_string()])
            .await
            .expect("set failed");

        let ws_id = workspace_agent::get(&state.db, &id)
            .await
            .expect("get failed")
            .expect("exists")
            .workspace_id;
        let listed = list(&state, json!({ "workspaceId": ws_id })).await.expect("list failed");
        let item = listed.as_array().unwrap().iter().find(|i| i["id"] == id).unwrap();
        assert_eq!(item["launchedSkillIds"].as_array().map(|a| a.len()), Some(1));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::instance`
Expected: FAIL to compile — `apply_skills_to_preamble` doesn't exist yet, and `instance.list`'s response has no `launchedSkillIds` key.

- [ ] **Step 3: Extract and implement `apply_skills_to_preamble`**

Add this private function above `pub async fn spawn` in `instance.rs`:

```rust
/// Compute this instance's skill content (builtin + attached custom, via
/// `repo::skill::content_for_agent`), write it to a per-instance sidecar file
/// if non-empty, and append ONE sanitized pointer sentence to `preamble` —
/// never the raw content, which may contain '\n'/'=' and would violate
/// `bootstrap_preamble`'s single-line/'='-free contract (ADR 0001). Persists
/// the launch snapshot (`session.launched_skill_ids`) unconditionally — an
/// empty attachment set still stores `"[]"`, distinct from a session that has
/// never launched at all (`NULL`).
///
/// Extracted out of `spawn`'s `cli` branch so it's unit-testable without
/// spawning a real PTY (this file's other tests avoid the `cli` dispatch
/// branch entirely — see `fixture_instance`'s doc comment).
async fn apply_skills_to_preamble(
    state: &AppState,
    agent_def_id: &str,
    instance_id: &str,
    session_id: &str,
    preamble: String,
) -> Result<String, AppError> {
    let (skill_body, skill_ids) = repo::skill::content_for_agent(&state.db, agent_def_id).await?;
    let preamble = if skill_body.is_empty() {
        preamble
    } else {
        let path = crate::engine::agentctx::write_skill_sidecar(instance_id, &skill_body)
            .map_err(|e| AppError::Internal(format!("write skill sidecar: {e}")))?;
        format!(
            "{preamble} {}",
            crate::engine::agentctx::skill_pointer_sentence(&path)
        )
    };
    repo::session::set_launched_skill_ids(&state.db, session_id, &skill_ids).await?;
    Ok(preamble)
}
```

- [ ] **Step 4: Call it from `spawn()`'s `cli` branch**

In `spawn()`, right after the existing preamble construction (the `bootstrap_preamble(...)` call), before `let mut launch = String::from(base);`:

```rust
            let preamble =
                apply_skills_to_preamble(state, &def.id, &id, &session.id, preamble).await?;

```

- [ ] **Step 5: Wire the `list()` handler**

Change the one line in `list()`:

```rust
    let rows = repo::workspace_agent::list_by_workspace_with_launched_skills(&state.db, &req.workspace_id).await?;
```

(replacing `repo::workspace_agent::list_by_workspace(&state.db, &req.workspace_id).await?`)

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::instance`
Expected: PASS (all existing + 3 new tests)

- [ ] **Step 7: Run full Rust baseline**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings && cargo fmt --manifest-path src-tauri/Cargo.toml --check`
Expected: all green. This is the last Rust task — a full green baseline here means the entire backend half of the feature works end-to-end at the unit-test level.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/engine/commands/instance.rs
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(commands): inject skills at cli spawn via sidecar file, snapshot + join

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 11: Frontend `ipc/types.ts` — `Skill`, `AgentDefinition.skillIds`, `WorkspaceAgent.launchedSkillIds`

**Files:**
- Modify: `src/ipc/types.ts`

**Interfaces:**
- Produces: `Skill` interface, `AgentDefinition.skillIds?: string[]`, `WorkspaceAgent.launchedSkillIds?: string[]` — consumed by every remaining frontend task.

- [ ] **Step 1: Add the `Skill` interface**

In `src/ipc/types.ts`, add (near `AgentDefinition`, e.g. right after its closing brace):

```ts
export interface Skill {
  id: string;
  name: string;
  description?: string;
  content: string;
  kind: "builtin" | "custom";
  icon?: string;
  /** Annotated by `skill.list`: how many AgentDefinitions have this attached. */
  attachedTo?: number;
}
```

- [ ] **Step 2: Add `skillIds` to `AgentDefinition`**

In the `AgentDefinition` interface, add after `contextWindow?: "1m" | "200k";`:

```ts
  /** Annotated by `agentDef.list`: attached CUSTOM skill ids (builtin skills
   *  are always active and are NOT listed here — see Skill's `kind`). */
  skillIds?: string[];
```

- [ ] **Step 3: Add `launchedSkillIds` to `WorkspaceAgent`**

In the `WorkspaceAgent` interface:

```ts
export interface WorkspaceAgent {
  id: string;
  workspaceId: string;
  agentDefId: string;
  status: "running" | "idle" | "waiting";
  addedAt: string;
  /** Annotated by `instance.list`: skill ids used at the last launch (see
   *  Session.launchedSkillIds — same value, joined in for the Roster). */
  launchedSkillIds?: string[];
}
```

- [ ] **Step 4: Verify it compiles**

Run: `pnpm exec tsc --noEmit`
Expected: no new errors (these are additive, optional fields).

- [ ] **Step 5: Commit**

```bash
git add src/ipc/types.ts
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(ipc): Skill type + skillIds/launchedSkillIds annotations

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 12: Frontend `ipc/commands.ts` — `skill.*` commands + bindings

**Files:**
- Modify: `src/ipc/commands.ts`

**Interfaces:**
- Consumes: `Skill` (Task 11).
- Produces: `ipc.skill.list()`, `ipc.skill.save(req)`, `ipc.skill.delete(req)` — consumed by Tasks 13-14.

- [ ] **Step 1: Import `Skill`**

In `src/ipc/commands.ts`'s import block, add `Skill` to the list imported from `./types`.

- [ ] **Step 2: Add the command map entries**

In the `Commands` interface, add after the `agentDef.addToWorkspace` entry:

```ts
  "skill.list": {
    req: void;
    res: Skill[];
  };
  "skill.save": {
    req: { id?: string; name: string; description?: string; content: string };
    res: Skill;
  };
  "skill.delete": {
    req: { id: string };
    res: void;
  };
```

- [ ] **Step 3: Add the `ipc.skill` bindings**

Find the object where `ipc.agentDef` is bound (near the bottom of the file, alongside `ipc.workspace`) and add a sibling `skill` object:

```ts
  skill: {
    list: () => call("skill.list"),
    save: (req: Commands["skill.save"]["req"]) => call("skill.save", req),
    delete: (req: Commands["skill.delete"]["req"]) => call("skill.delete", req),
  },
```

- [ ] **Step 4: Verify it compiles**

Run: `pnpm exec tsc --noEmit`
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/ipc/commands.ts
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(ipc): skill.list/save/delete command bindings

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 13: `SkillEditor.tsx` (new)

**Files:**
- Create: `src/components/SkillEditor.tsx`

**Interfaces:**
- Consumes: `ipc.skill.save` (Task 12), `Skill` (Task 11).
- Produces: `SkillEditor` component — consumed by Task 14.

- [ ] **Step 1: Write the component**

```tsx
import { useState } from "react";
import { X, Wand2 } from "lucide-react";
import { ipc } from "../ipc";
import type { Skill } from "../ipc";

export interface SkillEditorProps {
  onClose: () => void;
  onSaved: (skill: Skill) => void;
  /** Pre-fill the form for editing an existing CUSTOM skill. Never a builtin
   *  one — the Library never opens the editor for builtin cards. */
  initialSkill?: Skill;
}

/**
 * Create or edit a CUSTOM skill: name, short description (shown in Library
 * lists), and the full markdown `content` injected into a cli agent's skill
 * sidecar file at launch (see docs/adr/0001-skill-system-v1.md). Builtin
 * skills are never edited here.
 */
export function SkillEditor({ onClose, onSaved, initialSkill }: SkillEditorProps) {
  const isEditing = initialSkill !== undefined;
  const [name, setName] = useState(initialSkill?.name ?? "");
  const [description, setDescription] = useState(initialSkill?.description ?? "");
  const [content, setContent] = useState(initialSkill?.content ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSave() {
    if (!name.trim()) {
      setError("Name is required");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const skill = await ipc.skill.save({
        id: initialSkill?.id,
        name: name.trim(),
        description: description.trim() || undefined,
        content,
      });
      onSaved(skill);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30">
      <div className="w-[520px] max-h-[85vh] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]">
        <div className="h-11 flex items-center justify-between px-4 border-b border-overlay/[0.06] shrink-0">
          <div className="flex items-center gap-2">
            <Wand2 className="w-4 h-4 text-accent" />
            <span className="text-[13px] font-semibold tracking-tight">
              {isEditing ? "Edit skill" : "New skill"}
            </span>
          </div>
          <button
            onClick={onClose}
            disabled={saving}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary disabled:opacity-50"
            aria-label="Close"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        <div className="p-5 overflow-y-auto scroll-thin flex-1">
          <div className="mb-4">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Name
            </div>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Code Reviewer"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-9 text-[13px] outline-none focus:ring-accent/50"
            />
          </div>

          <div className="mb-4">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Description
            </div>
            <input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Shown in the Skill Library list"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-9 text-[13px] outline-none focus:ring-accent/50"
            />
          </div>

          <div>
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Content
            </div>
            <textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              placeholder="Markdown instructions injected into the agent's skill sidecar file at launch"
              rows={12}
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 py-2 text-[12.5px] font-mono outline-none focus:ring-accent/50 resize-none"
            />
          </div>

          {error && <p className="text-[12px] text-danger mt-3">{error}</p>}
        </div>

        <div className="border-t border-overlay/[0.07] px-5 py-3 bg-surface shrink-0 flex items-center gap-2">
          <button
            onClick={onClose}
            disabled={saving}
            className="flex-1 text-[12.5px] font-medium text-text-secondary bg-surface ring-1 ring-overlay/[0.08] rounded-lg py-2.5 hover:bg-overlay/[0.02] disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="flex-[1.4] text-[12.5px] font-semibold text-white bg-accent rounded-lg py-2.5 hover:brightness-105 disabled:opacity-50"
          >
            {saving ? "Saving…" : "Save skill"}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify it compiles**

Run: `pnpm exec tsc --noEmit`
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/components/SkillEditor.tsx
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(ui): add SkillEditor modal for creating/editing custom skills

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 14: `SkillLibrary.tsx` (new)

**Files:**
- Create: `src/components/SkillLibrary.tsx`

**Interfaces:**
- Consumes: `ipc.skill.{list,delete}` (Task 12), `Skill` (Task 11), `SkillEditor` (Task 13).
- Produces: `SkillLibrary` component — consumed by Task 15.

- [ ] **Step 1: Write the component**

Mirrors `Library.tsx`'s structure closely (sheet panel, search, two-step delete confirm on cards) but splits into a read-only "System" section and a full-CRUD "Custom" section:

```tsx
import { useEffect, useState } from "react";
import { Wand2, Search, Plus, Pencil, Trash2, X } from "lucide-react";
import { ipc } from "../ipc";
import type { Skill } from "../ipc";
import { SkillEditor } from "./SkillEditor";

export interface SkillLibraryProps {
  onClose: () => void;
}

interface CustomSkillCardProps {
  skill: Skill;
  onEdit: () => void;
  onDelete: () => void;
  deleting: boolean;
}

function CustomSkillCard({ skill, onEdit, onDelete, deleting }: CustomSkillCardProps) {
  const [confirming, setConfirming] = useState(false);
  const count = skill.attachedTo ?? 0;
  const countLabel = count === 0 ? "Not attached to any agent" : `attached to ${count} agent${count !== 1 ? "s" : ""}`;

  return (
    <div className="rounded-xl p-3.5 ring-hair bg-surface">
      <div className="flex items-start gap-3">
        <div className="w-10 h-10 rounded-[11px] bg-accent/[0.12] text-accent grid place-items-center shrink-0">
          <Wand2 className="w-5 h-5" />
        </div>
        <div className="flex-1 min-w-0">
          <span className="text-[13.5px] font-semibold">{skill.name}</span>
          <div className="text-[11px] text-text-muted truncate">{skill.description || "No description"}</div>
          <div className="text-[10.5px] text-text-muted mt-1">{countLabel}</div>
        </div>
      </div>
      <div className="flex items-center gap-1.5 mt-3">
        <button
          onClick={onEdit}
          className="flex-1 text-[11.5px] font-medium text-text-body bg-surface ring-hair rounded-lg py-1.5 hover:bg-overlay/[0.02] flex items-center justify-center gap-1"
        >
          <Pencil className="w-3.5 h-3.5" />
          Edit
        </button>
        {confirming ? (
          <button
            onClick={onDelete}
            disabled={deleting}
            onMouseLeave={() => setConfirming(false)}
            className="flex-1 text-[11.5px] font-semibold text-white bg-danger rounded-lg py-1.5 hover:brightness-105 disabled:opacity-50 flex items-center justify-center gap-1"
          >
            <Trash2 className="w-3.5 h-3.5" />
            {deleting ? "Deleting…" : "Confirm"}
          </button>
        ) : (
          <button
            onClick={() => setConfirming(true)}
            className="flex-1 text-[11.5px] font-medium text-danger bg-danger/[0.06] rounded-lg py-1.5 hover:bg-danger/10 flex items-center justify-center gap-1"
          >
            <Trash2 className="w-3.5 h-3.5" />
            Delete
          </button>
        )}
      </div>
    </div>
  );
}

function SystemSkillCard({ skill }: { skill: Skill }) {
  return (
    <div className="rounded-xl p-3.5 ring-hair bg-surface opacity-80">
      <div className="flex items-start gap-3">
        <div className="w-10 h-10 rounded-[11px] bg-overlay/[0.06] text-text-secondary grid place-items-center shrink-0">
          <Wand2 className="w-5 h-5" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-[13.5px] font-semibold">{skill.name}</span>
            <span className="text-[9.5px] font-medium text-text-muted bg-overlay/[0.05] px-1.5 py-px rounded">
              Always on
            </span>
          </div>
          <div className="text-[11px] text-text-muted truncate">{skill.description || "No description"}</div>
        </div>
      </div>
    </div>
  );
}

export function SkillLibrary({ onClose }: SkillLibraryProps) {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loadError, setLoadError] = useState(false);
  const [search, setSearch] = useState("");
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [editingSkill, setEditingSkill] = useState<Skill | undefined>(undefined);
  const [showEditor, setShowEditor] = useState(false);

  async function loadSkills() {
    try {
      setSkills(await ipc.skill.list());
      setLoadError(false);
    } catch (err: unknown) {
      if (import.meta.env.DEV) console.error("SkillLibrary: skill.list failed", err);
      setSkills([]);
      setLoadError(true);
    }
  }

  useEffect(() => {
    loadSkills();
  }, []);

  async function handleDelete(id: string) {
    setDeletingId(id);
    try {
      await ipc.skill.delete({ id });
      await loadSkills();
    } catch (err: unknown) {
      if (import.meta.env.DEV) console.error("SkillLibrary: skill.delete failed", err);
    } finally {
      setDeletingId(null);
    }
  }

  const q = search.trim().toLowerCase();
  const matches = (s: Skill) =>
    !q || s.name.toLowerCase().includes(q) || (s.description ?? "").toLowerCase().includes(q);
  const systemSkills = skills.filter((s) => s.kind === "builtin" && matches(s));
  const customSkills = skills.filter((s) => s.kind === "custom" && matches(s));

  return (
    <div className="fixed inset-0 z-40 flex justify-end">
      <div className="absolute inset-0 bg-black/30" onClick={onClose} />

      <div className="relative w-[440px] max-w-full h-full bg-sidebar shadow-2xl flex flex-col ring-1 ring-overlay/[0.08]">
        <div className="h-12 flex items-center gap-2 px-4 border-b border-overlay/[0.06] shrink-0">
          <Wand2 className="w-[15px] h-[15px] text-accent shrink-0" />
          <span className="text-[13px] font-semibold tracking-tight">Skill Library</span>
          <button
            onClick={onClose}
            className="ml-auto w-6 h-6 grid place-items-center rounded-md hover:bg-overlay/[0.06] text-text-muted shrink-0"
            aria-label="Close Skill Library"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>

        <div className="px-3 pt-3 pb-2 shrink-0">
          <div className="flex items-center gap-2 bg-overlay/[0.05] rounded-lg px-2.5 h-7">
            <Search className="w-[13px] h-[13px] text-text-muted shrink-0" />
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search skills"
              className="bg-transparent outline-none text-[12px] placeholder:text-text-tertiary w-full"
            />
          </div>
        </div>

        <div className="flex-1 overflow-y-auto scroll-thin px-3 pb-3 space-y-4">
          {loadError ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-center px-6">
              <Wand2 className="w-9 h-9 text-text-quaternary" />
              <p className="text-[13px] font-semibold text-text-secondary">Failed to load skills</p>
              <p className="text-[11.5px] text-text-tertiary">Check the app is running and try again</p>
            </div>
          ) : (
            <>
              {systemSkills.length > 0 && (
                <div>
                  <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 px-0.5">
                    System
                  </div>
                  <div className="space-y-2">
                    {systemSkills.map((s) => (
                      <SystemSkillCard key={s.id} skill={s} />
                    ))}
                  </div>
                </div>
              )}
              <div>
                <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 px-0.5">
                  Custom
                </div>
                {customSkills.length === 0 ? (
                  <p className="text-[11.5px] text-text-tertiary px-0.5">
                    {skills.length === 0 ? "No skills yet" : "No matching custom skills"}
                  </p>
                ) : (
                  <div className="space-y-2">
                    {customSkills.map((s) => (
                      <CustomSkillCard
                        key={s.id}
                        skill={s}
                        onEdit={() => {
                          setEditingSkill(s);
                          setShowEditor(true);
                        }}
                        onDelete={() => handleDelete(s.id)}
                        deleting={deletingId === s.id}
                      />
                    ))}
                  </div>
                )}
              </div>
            </>
          )}
        </div>

        <div className="border-t border-overlay/[0.06] p-2 shrink-0">
          <button
            onClick={() => {
              setEditingSkill(undefined);
              setShowEditor(true);
            }}
            className="w-full flex items-center justify-center gap-1.5 px-2 py-2 rounded-lg bg-accent text-white hover:brightness-105"
          >
            <Plus className="w-4 h-4" />
            <span className="text-[12.5px] font-semibold">New skill</span>
          </button>
        </div>
      </div>

      {showEditor && (
        <SkillEditor
          initialSkill={editingSkill}
          onClose={() => setShowEditor(false)}
          onSaved={() => {
            setShowEditor(false);
            loadSkills();
          }}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 2: Verify it compiles**

Run: `pnpm exec tsc --noEmit`
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/components/SkillLibrary.tsx
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(ui): add SkillLibrary sheet (system read-only + custom CRUD)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 15: Wire `SkillLibrary` into `Rail` + `AppShell`

**Files:**
- Modify: `src/components/Rail.tsx`
- Modify: `src/components/AppShell.tsx`

**Interfaces:**
- Consumes: `SkillLibrary` (Task 14).
- Produces: a Rail icon that opens the Skill Library sheet.

- [ ] **Step 1: Add the Rail icon**

In `src/components/Rail.tsx`, add `Wand2` to the `lucide-react` import, add `onOpenSkillLibrary?: () => void` to `RailProps`, thread it into the destructured props, and add a button next to the existing "Agent Library" button (inside the `mt-auto` bottom-actions div):

```tsx
        <button
          className="w-9 h-9 rounded-[10px] bg-surface ring-hair text-text-body grid place-items-center hover:bg-overlay/[0.03]"
          title="Skill Library"
          onClick={onOpenSkillLibrary}
        >
          <Wand2 className="w-[17px] h-[17px]" />
        </button>
```

placed right after the existing "Agent Library" button's closing `</button>`.

- [ ] **Step 2: Wire it in `AppShell.tsx`**

Add state near the existing `showLibrary` state:

```tsx
  const [showSkillLibrary, setShowSkillLibrary] = useState(false);
```

Import `SkillLibrary`:

```tsx
import { SkillLibrary } from "./SkillLibrary";
```

Pass the prop to `<Rail>` alongside `onOpenLibrary`:

```tsx
          onOpenSkillLibrary={() => setShowSkillLibrary(true)}
```

Render the sheet alongside the existing `{showLibrary && <Library .../>}` block:

```tsx
      {showSkillLibrary && <SkillLibrary onClose={() => setShowSkillLibrary(false)} />}
```

- [ ] **Step 3: Verify it compiles**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: no errors; build succeeds.

- [ ] **Step 4: Manual smoke test**

Run the dev app (`pnpm tauri dev` or the project's usual dev command), click the new Rail icon, confirm the Skill Library sheet opens, "New skill" opens the editor, saving a skill shows it under "Custom", and deleting requires the two-step confirm. This is a UI behavior change — verify it in the running app, don't just trust `tsc`.

- [ ] **Step 5: Commit**

```bash
git add src/components/Rail.tsx src/components/AppShell.tsx
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(ui): add Rail icon opening the Skill Library

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 16: `Builder.tsx` — System/Custom skill sections

**Files:**
- Modify: `src/components/Builder.tsx`

**Interfaces:**
- Consumes: `ipc.skill.list` (Task 12), `Skill` (Task 11).
- Produces: `skillIds` now flows from Builder's UI into `agentDef.save`.

- [ ] **Step 1: Fetch skills on mount**

Add state and an effect near the other `useState` declarations (around where `initialDef` is destructured):

```tsx
  const [allSkills, setAllSkills] = useState<Skill[]>([]);
  const [skillIds, setSkillIds] = useState<string[]>(initialDef?.skillIds ?? []);

  useEffect(() => {
    ipc.skill
      .list()
      .then(setAllSkills)
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("Builder: skill.list failed", err);
      });
  }, []);
```

Add `useEffect` to the existing `import { useState } from "react";` line (change to `import { useEffect, useState } from "react";`), and add `Skill` to the `import type { AgentDefinition } from "../ipc";` line (change to `import type { AgentDefinition, Skill } from "../ipc";`).

- [ ] **Step 2: Include `skillIds` in the save payload**

In `handleSave()`'s `ipc.agentDef.save({...})` call, add (skills are `cli`-only in v1 — omit for other types so a `chat`/`orchestrator` save never sends a stale list):

```ts
        skillIds: agentType === "cli" ? skillIds : undefined,
```

- [ ] **Step 3: Add the UI section**

Add a new `<section>` inside the `showCliConfig` block (right after the existing "Model" quick-presets `<div className="flex flex-wrap gap-1.5 mt-2">...</div>` closes, still inside the CLI config `<div className="rounded-xl ring-1 ...">`), OR as its own top-level `{showCliConfig && (...)}`-gated section right after the whole CLI config `</section>` closes — the latter is simpler and keeps the CLI launch config visually separate from skills:

```tsx
          {showCliConfig && (
            <section>
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
                Skills
              </div>
              <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]">
                {allSkills.filter((s) => s.kind === "builtin").length > 0 && (
                  <div className="px-3 py-2">
                    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                      System skills — always on
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {allSkills
                        .filter((s) => s.kind === "builtin")
                        .map((s) => (
                          <span
                            key={s.id}
                            className="text-[11px] font-medium px-2 py-0.5 rounded-md ring-1 ring-overlay/[0.08] text-text-secondary"
                          >
                            {s.name}
                          </span>
                        ))}
                    </div>
                  </div>
                )}
                <div className="px-3 py-2">
                  <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                    Custom skills
                  </div>
                  {allSkills.filter((s) => s.kind === "custom").length === 0 ? (
                    <p className="text-[11.5px] text-text-tertiary">
                      No custom skills yet — create one in the Skill Library.
                    </p>
                  ) : (
                    <div className="space-y-1">
                      {allSkills
                        .filter((s) => s.kind === "custom")
                        .map((s) => {
                          const checked = skillIds.includes(s.id);
                          return (
                            <label
                              key={s.id}
                              className="flex items-center gap-2 text-[12.5px] text-text-secondary cursor-pointer"
                            >
                              <input
                                type="checkbox"
                                checked={checked}
                                onChange={(e) =>
                                  setSkillIds((prev) =>
                                    e.target.checked ? [...prev, s.id] : prev.filter((id) => id !== s.id),
                                  )
                                }
                              />
                              {s.name}
                            </label>
                          );
                        })}
                    </div>
                  )}
                </div>
              </div>
            </section>
          )}
```

- [ ] **Step 4: Verify it compiles**

Run: `pnpm exec tsc --noEmit`
Expected: no errors.

- [ ] **Step 5: Manual smoke test**

Run the dev app, open Builder for a `cli`-type agent, confirm the "Skills" section appears (system skills as static badges, custom skills as checkboxes), toggle a checkbox, save, re-open the same agent, and confirm the checkbox state persisted.

- [ ] **Step 6: Run full frontend baseline**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: both green.

- [ ] **Step 7: Commit**

```bash
git add src/components/Builder.tsx
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(ui): Builder shows system skills (always on) + custom skill checklist

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 17: `Roster.tsx` — "Restart to apply" staleness badge

**Files:**
- Modify: `src/components/Roster.tsx`

**Interfaces:**
- Consumes: `WorkspaceAgent.launchedSkillIds` (Task 11), `AgentDefinition.skillIds` (Task 11).
- Produces: a visible badge on any running `cli` agent whose attached skills have drifted from what its live session last launched with.

- [ ] **Step 1: Compute staleness when building roster entries**

Add a `skillsStale: boolean` field to `RosterEntry`:

```ts
interface RosterEntry {
  instanceId: string;
  name: string;
  color: string;
  type: AgentDefinition["type"];
  status: WorkspaceAgent["status"];
  meta: string;
  /** True when the def's CURRENT skill attachments differ from what the live
   *  session actually launched with — see Session.launchedSkillIds. Only
   *  meaningful for `type === "cli"` (the only type skills apply to in v1). */
  skillsStale: boolean;
}
```

Add a helper above the component (near `deriveMeta`):

```ts
// A `cli` instance is "stale" when its definition's current skill ids differ
// from what its session actually launched with. Order matters (mirrors
// repo::skill::content_for_agent's deterministic ordering), so this is a
// straight array comparison, not a set comparison — reordering also counts as
// drift, matching the "content actually differs" intent. `undefined`
// launchedSkillIds (never launched yet) is never stale — nothing to compare
// against.
function computeSkillsStale(def: AgentDefinition, inst: WorkspaceAgent): boolean {
  if (def.type !== "cli") return false;
  if (inst.launchedSkillIds === undefined) return false;
  const current = def.skillIds ?? [];
  const launched = inst.launchedSkillIds;
  if (current.length !== launched.length) return true;
  return current.some((id, i) => id !== launched[i]);
}
```

Note: this compares CUSTOM skill ids only (both sides only ever carry custom ids per Task 9/10's contracts) — builtin skills are always active on both sides identically, so they never contribute to drift and are correctly excluded from both `def.skillIds` and `inst.launchedSkillIds`.

In the `Promise.all([...]).then(([instances, defs]) => {...})` block, add the field:

```ts
          rosterEntries.push({
            instanceId: inst.id,
            name: def.name,
            color: def.color ?? "#6e6e73",
            type: def.type,
            status: inst.status,
            meta: deriveMeta(def),
            skillsStale: computeSkillsStale(def, inst),
          });
```

- [ ] **Step 2: Render the badge in `AgentRow`**

In `AgentRow`'s JSX, find where the status dot renders (the element using `statusColor`) and add a sibling badge right after it, gated on `entry.skillsStale`:

```tsx
        {entry.skillsStale && (
          <span
            className="text-[9px] font-semibold text-warning bg-warning/[0.1] px-1.5 py-px rounded-md shrink-0"
            title="This agent's skills changed since it last launched — restart to apply"
          >
            Restart to apply
          </span>
        )}
```

- [ ] **Step 3: Verify it compiles**

Run: `pnpm exec tsc --noEmit`
Expected: no errors.

- [ ] **Step 4: Manual smoke test**

Run the dev app: launch a `cli` agent (so its session has a `launchedSkillIds` snapshot), then open Builder for that same agent's definition and toggle a custom skill on/off and save. Confirm the Roster row for the still-running instance now shows "Restart to apply", and that removing the agent + re-adding it (fresh instance, fresh session, no snapshot yet) shows no badge before its first launch.

- [ ] **Step 5: Run full frontend baseline**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: both green.

- [ ] **Step 6: Commit**

```bash
git add src/components/Roster.tsx
git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit -m "$(cat <<'EOF'
feat(ui): Roster shows Restart to apply when a running agent's skills drift

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Final verification (after all 17 tasks)

- [ ] Run the full Rust baseline once more from repo root:
  `cargo test --manifest-path src-tauri/Cargo.toml --lib && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings && cargo fmt --manifest-path src-tauri/Cargo.toml --check`
- [ ] Run the full frontend baseline once more:
  `pnpm exec tsc --noEmit && pnpm build`
- [ ] Walk every acceptance criterion in `docs/specs/2026-07-01-skill-system-design.md`'s "Acceptance criteria" section and check it off against the running app, not just the test suite (per this session's established lesson: UI behavior claims need empirical verification, not just static code review).

//! Database connection management and migration runner.
//!
//! Entry points:
//! - [`connect`]           — on-disk pool for production use
//! - [`connect_in_memory`] — in-memory pool for tests
//!
//! Migration strategy: `PRAGMA user_version` is the version counter.
//! Each milestone appends a new version gate inside [`migrate`].

use std::path::PathBuf;
use std::time::Duration;
// `FromStr` is only needed by the in-memory pool helpers, which are test-only.
#[cfg(test)]
use std::str::FromStr;

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};

/// Returns `~/Library/Application Support/Conclave/conclave.db` (macOS).
/// Creates the parent directory if it does not exist.
///
/// # Panics
/// Panics if the user data directory cannot be resolved (`dirs::data_dir()`
/// returns `None`) or the `Conclave` directory cannot be created. Both are
/// unrecoverable at startup on a supported macOS install.
pub fn db_path() -> PathBuf {
    let dir = dirs::data_dir()
        .expect("could not resolve user data directory")
        .join("Conclave");
    std::fs::create_dir_all(&dir).expect("could not create Conclave data directory");
    dir.join("conclave.db")
}

/// Opens the on-disk SQLite database as a connection pool, applies PRAGMAs,
/// and runs pending migrations.
///
/// # Errors
/// Returns [`sqlx::Error`] if the database file cannot be opened or migrated.
pub async fn connect() -> sqlx::Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(db_path())
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePoolOptions::new().connect_with(opts).await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// Idempotent migration runner.
///
/// Reads `PRAGMA user_version` and applies pending migrations in order.
/// Each migration runs inside an explicit transaction, and the
/// `user_version` bump is issued INSIDE that transaction (SQLite makes
/// `user_version` transactional). This makes apply-and-bump atomic: a crash
/// mid-migration rolls back the DDL *and* the version, so the schema and the
/// version counter can never disagree — re-running on next launch is safe.
///
/// **Adding future migrations**: append another `if version < N { … }` block.
pub(crate) async fn migrate(pool: &SqlitePool) -> sqlx::Result<()> {
    // Read the version INSIDE the transaction so check-and-bump is atomic —
    // no TOCTOU window between two separate pool checkouts.
    let mut tx = pool.begin().await?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *tx)
        .await?;

    if version < 1 {
        sqlx::raw_sql(include_str!("migrations/0001_init.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 1;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 2 {
        sqlx::raw_sql(include_str!("migrations/0002_seed_core_tools.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 2;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 3 {
        sqlx::raw_sql(include_str!("migrations/0003_agent_cli_config.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 3;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 4 {
        sqlx::raw_sql(include_str!("migrations/0004_skill_system.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 4;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 5 {
        sqlx::raw_sql(include_str!("migrations/0005_drop_skill_kind.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 5;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 6 {
        sqlx::raw_sql(include_str!("migrations/0006_selected_builtin_skills.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 6;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 7 {
        sqlx::raw_sql(include_str!("migrations/0007_workspace_hidden.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 7;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 8 {
        sqlx::raw_sql(include_str!("migrations/0008_role_system.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 8;")
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Opens an in-memory SQLite pool suitable for unit tests.
/// Uses `max_connections(1)` to prevent sqlx from opening multiple
/// connections — each `:memory:` connection is a separate database, so
/// with more than one connection the schema from migration would be
/// invisible to queries hitting a different connection.
///
/// # Panics
/// Panics if the in-memory database cannot be opened or migrated.
#[cfg(test)]
pub(crate) async fn connect_in_memory() -> SqlitePool {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")
        .expect("invalid in-memory connection string")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("failed to open in-memory pool");
    migrate(&pool)
        .await
        .expect("failed to migrate in-memory db");
    pool
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 19 entity tables must exist after migration.
    #[tokio::test]
    async fn migrate_creates_all_tables() {
        let pool = connect_in_memory().await;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master \
             WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_one(&pool)
        .await
        .expect("table-count query failed");
        assert_eq!(count, 20, "expected 20 tables, got {count}");
    }

    /// Running migrate twice must not error and must leave user_version == 6.
    #[tokio::test]
    async fn migrate_is_idempotent() {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("open failed");

        migrate(&pool).await.expect("first migration failed");
        migrate(&pool)
            .await
            .expect("second migration must be a no-op");

        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master \
             WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        )
        .fetch_one(&pool)
        .await
        .expect("table-count query failed");
        assert_eq!(
            count, 20,
            "expected 20 tables after idempotent run, got {count}"
        );

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("user_version query failed");
        assert_eq!(version, 8, "user_version should be 8");

        // The seed migration must not duplicate rows across an idempotent run.
        let tool_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM tool WHERE id = 'tool-conclave'")
                .fetch_one(&pool)
                .await
                .expect("tool-count query failed");
        assert_eq!(
            tool_count, 1,
            "seed must not duplicate after idempotent run"
        );
    }

    /// Migration 0002 seeds exactly one `tool-conclave` core row; running
    /// migrate again leaves exactly one row (INSERT OR IGNORE is idempotent).
    #[tokio::test]
    async fn migrate_seeds_core_conclave_tool() {
        let pool = connect_in_memory().await;

        let (id, is_core, kind): (String, i64, String) =
            sqlx::query_as("SELECT id, is_core, kind FROM tool WHERE id = 'tool-conclave'")
                .fetch_one(&pool)
                .await
                .expect("conclave tool row should exist after migration");

        assert_eq!(id, "tool-conclave");
        assert_eq!(is_core, 1, "is_core must be 1");
        assert_eq!(kind, "builtin");

        // Second migrate run — seed must stay at exactly one row.
        migrate(&pool)
            .await
            .expect("second migrate must be a no-op");

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tool WHERE id = 'tool-conclave'")
            .fetch_one(&pool)
            .await
            .expect("count query failed");
        assert_eq!(count, 1, "INSERT OR IGNORE must not duplicate the row");
    }

    /// With `foreign_keys(true)`, inserting a `workspace_agent` row that
    /// references a non-existent workspace must fail.
    #[tokio::test]
    async fn foreign_keys_enforced() {
        let pool = connect_in_memory().await;
        let result = sqlx::query(
            "INSERT INTO workspace_agent(id, workspace_id, agent_def_id, status, added_at) \
             VALUES ('wa-1', 'no-such-workspace', 'no-such-agent', 'idle', '2024-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await;
        assert!(
            result.is_err(),
            "FK violation against workspace should return an error"
        );
    }

    /// Smoke-test that chain-builder is linked and produces correct SQLite SQL.
    #[tokio::test]
    async fn chain_builder_builds_select() {
        use chain_builder::{QueryBuilder, Sqlite};
        let (sql, binds) = QueryBuilder::<Sqlite>::table("workspace")
            .select(["id", "name"])
            .where_eq("id", "w1")
            .to_sql();
        assert!(
            sql.contains("workspace"),
            "sql should reference table: {sql}"
        );
        assert!(sql.contains('?'), "sqlite should use ? placeholders: {sql}");
        assert_eq!(binds.len(), 1, "one bind for the id filter");
    }

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
        assert_eq!(version, 8);
    }

    /// Migration 0005 drops `skill.kind` entirely — builtin skills now come
    /// from a bundled folder, never the DB (see ADR 0002). Every `skill` row
    /// after this migration is, structurally, a custom skill.
    #[tokio::test]
    async fn migrate_drops_skill_kind_column() {
        let pool = connect_in_memory().await;

        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('skill')")
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

    /// Migration 0007 added `workspace.hidden`, defaulting existing and new
    /// rows to visible (`0`) so hidden agent-assist workspaces are opt-in.
    #[tokio::test]
    async fn migrate_adds_workspace_hidden_column() {
        let pool = connect_in_memory().await;

        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('workspace')")
                .fetch_all(&pool)
                .await
                .expect("pragma_table_info query failed");
        assert!(
            columns.iter().any(|c| c == "hidden"),
            "workspace.hidden must exist after migration: {columns:?}"
        );

        // An insert with no `hidden` column reference must default to 0.
        sqlx::query(
            "INSERT INTO workspace (id, name, folder_path, created_at) \
             VALUES ('ws-no-hidden', 'WS', '/tmp/ws', '2024-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert without hidden should succeed");

        let hidden: i64 =
            sqlx::query_scalar("SELECT hidden FROM workspace WHERE id = 'ws-no-hidden'")
                .fetch_one(&pool)
                .await
                .expect("select failed");
        assert_eq!(hidden, 0);
    }

    #[tokio::test]
    async fn migrate_adds_selected_builtin_skill_ids_column() {
        let pool = connect_in_memory().await;

        // Column exists and accepts a JSON array of ids.
        sqlx::query(
            "INSERT INTO agent_definition (id, name, type, harness_mode, created_at, selected_builtin_skill_ids) \
             VALUES ('a1', 'A', 'cli', 'own', '2024-01-01T00:00:00Z', '[\"opt-1\",\"opt-2\"]')",
        )
        .execute(&pool)
        .await
        .expect("insert with selected_builtin_skill_ids should succeed");

        let stored: String = sqlx::query_scalar(
            "SELECT selected_builtin_skill_ids FROM agent_definition WHERE id = 'a1'",
        )
        .fetch_one(&pool)
        .await
        .expect("select failed");
        assert_eq!(stored, "[\"opt-1\",\"opt-2\"]");

        // NULL (no selection) must also be a valid, common state.
        sqlx::query("INSERT INTO agent_definition (id, name, type, harness_mode, created_at) VALUES ('a2', 'B', 'cli', 'own', '2024-01-01T00:00:00Z')")
            .execute(&pool)
            .await
            .expect("insert without selected_builtin_skill_ids should succeed");

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("pragma failed");
        assert_eq!(version, 8);
    }

    /// Migration 0008 adds the `role` table (ADR 0005) and
    /// `agent_definition.role_id`. Roles are first-class bundles; builtin roles
    /// ship as a bundled folder so this table holds only custom roles.
    #[tokio::test]
    async fn migrate_adds_role_table_and_role_id_column() {
        let pool = connect_in_memory().await;

        // The role table exists and `skill_ids` defaults to '[]'.
        sqlx::query(
            "INSERT INTO role (id, name, description, created_at) \
             VALUES ('r1', 'Custom', 'A custom role.', '2024-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert role should succeed");
        let skill_ids: String =
            sqlx::query_scalar("SELECT skill_ids FROM role WHERE id = 'r1'")
                .fetch_one(&pool)
                .await
                .expect("select failed");
        assert_eq!(skill_ids, "[]");

        // agent_definition.role_id exists and defaults to NULL.
        sqlx::query(
            "INSERT INTO agent_definition (id, name, type, harness_mode, created_at) \
             VALUES ('ad1', 'A', 'cli', 'own', '2024-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert agent_definition should succeed");
        let role_id: Option<String> =
            sqlx::query_scalar("SELECT role_id FROM agent_definition WHERE id = 'ad1'")
                .fetch_one(&pool)
                .await
                .expect("select failed");
        assert!(role_id.is_none());
    }
}

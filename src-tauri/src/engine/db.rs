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

    if version < 9 {
        sqlx::raw_sql(include_str!("migrations/0009_memory_system.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 9;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 10 {
        sqlx::raw_sql(include_str!("migrations/0010_memory_search_index.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 10;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 11 {
        sqlx::raw_sql(include_str!("migrations/0011_seed_memory_tool.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 11;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 12 {
        sqlx::raw_sql(include_str!("migrations/0012_task_system.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 12;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 13 {
        sqlx::raw_sql(include_str!("migrations/0013_memory_proposal.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 13;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 14 {
        sqlx::raw_sql(include_str!("migrations/0014_artifact_workspace.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 14;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 15 {
        sqlx::raw_sql(include_str!("migrations/0015_position_system.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 15;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 16 {
        sqlx::raw_sql(include_str!("migrations/0016_agent_default_level.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 16;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 17 {
        sqlx::raw_sql(include_str!("migrations/0017_agent_rtk_enabled.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 17;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 18 {
        sqlx::raw_sql(include_str!("migrations/0018_task_event_plan_check.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 18;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 19 {
        sqlx::raw_sql(include_str!("migrations/0019_proxy_metric.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 19;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 20 {
        sqlx::raw_sql(include_str!("migrations/0020_agent_proxy_enabled.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 20;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 21 {
        sqlx::raw_sql(include_str!("migrations/0021_proxy_checkpoint_metric.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 21;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 22 {
        sqlx::raw_sql(include_str!("migrations/0022_proxy_checkpoint_outcome.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 22;")
            .execute(&mut *tx)
            .await?;
    }

    if version < 23 {
        sqlx::raw_sql(include_str!(
            "migrations/0023_proxy_checkpoint_error_snippet.sql"
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::raw_sql("PRAGMA user_version = 23;")
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

    /// Build an in-memory DB at schema **v13** — the state just before 0014 —
    /// by applying the migration chain 0001..0013 exactly as [`migrate`] would
    /// (one transaction, `user_version` bumped inside it), then stopping. Lets
    /// a test exercise the REAL 0014 upgrade against the OLD `artifact` schema,
    /// which `connect_in_memory` (already at v15) cannot.
    async fn connect_at_v13() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("invalid in-memory connection string")
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("failed to open in-memory pool");
        let mut tx = pool.begin().await.expect("begin v13 setup");
        for sql in [
            include_str!("migrations/0001_init.sql"),
            include_str!("migrations/0002_seed_core_tools.sql"),
            include_str!("migrations/0003_agent_cli_config.sql"),
            include_str!("migrations/0004_skill_system.sql"),
            include_str!("migrations/0005_drop_skill_kind.sql"),
            include_str!("migrations/0006_selected_builtin_skills.sql"),
            include_str!("migrations/0007_workspace_hidden.sql"),
            include_str!("migrations/0008_role_system.sql"),
            include_str!("migrations/0009_memory_system.sql"),
            include_str!("migrations/0010_memory_search_index.sql"),
            include_str!("migrations/0011_seed_memory_tool.sql"),
            include_str!("migrations/0012_task_system.sql"),
            include_str!("migrations/0013_memory_proposal.sql"),
        ] {
            sqlx::raw_sql(sql)
                .execute(&mut *tx)
                .await
                .expect("apply pre-0014 migration");
        }
        sqlx::raw_sql("PRAGMA user_version = 13;")
            .execute(&mut *tx)
            .await
            .expect("set user_version = 13");
        tx.commit().await.expect("commit v13 setup");
        pool
    }

    /// The 0014 upgrade path itself: a real pre-0014 `artifact` row (owned by a
    /// message, body in `html`) must survive the INSERT..SELECT/DROP/RENAME
    /// rebuild — folded into `content` as `kind='html'`, with id/message_id/
    /// filename/sandboxed intact and no FK left dangling. This is the test the
    /// master plan (docs/2026-07-05-plan-design-artifacts-views.md:72-75)
    /// requires; the earlier repo-level test only checked the post-migration
    /// shape and could not catch a lossy rebuild (Armin, challenge 2a190c1a).
    #[tokio::test]
    async fn migrate_0014_preserves_legacy_artifact_rows() {
        use crate::engine::repo::workspace;

        let pool = connect_at_v13().await;

        // Build a real message to own the legacy artifact (message_id was NOT
        // NULL pre-0014). Seed the pre-0014 schema directly: modern repo
        // helpers now assume later additive columns like
        // `agent_definition.default_level`, which do not exist at v13.
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace");
        let def_id = "def-v13";
        sqlx::query(
            "INSERT INTO agent_definition (id, name, type, model, harness_mode, created_at) \
             VALUES (?1, 'A', 'chat', 'm', 'own', '2020-01-01T00:00:00+00:00')",
        )
        .bind(def_id)
        .execute(&pool)
        .await
        .expect("seed agent_def");
        let wa_id = "wa-v13";
        sqlx::query(
            "INSERT INTO workspace_agent (id, workspace_id, agent_def_id, status, added_at) \
             VALUES (?1, ?2, ?3, 'idle', '2020-01-01T00:00:00+00:00')",
        )
        .bind(wa_id)
        .bind(&ws.id)
        .bind(def_id)
        .execute(&pool)
        .await
        .expect("seed workspace_agent");
        let sid = "session-v13";
        sqlx::query(
            "INSERT INTO session \
             (id, workspace_agent_id, context_tokens, context_limit, started_at, last_active_at) \
             VALUES (?1, ?2, 0, ?3, '2020-01-01T00:00:00+00:00', NULL)",
        )
        .bind(sid)
        .bind(wa_id)
        // v13 seed for a chat definition (cli_kind NULL) — the resolver's
        // conservative branch, so the constant is the correct stamp here.
        .bind(crate::engine::repo::session::DEFAULT_CONTEXT_LIMIT)
        .execute(&pool)
        .await
        .expect("seed session");

        sqlx::query(
            "INSERT INTO message (id, session_id, role, text, created_at) \
                 VALUES ('msg-1', ?1, 'agent', 'hello', '2020-01-01T00:00:00+00:00')",
        )
        .bind(sid)
        .execute(&pool)
        .await
        .expect("seed message");

        // The pre-0014 artifact: message-owned, body in `html`, sandboxed=1.
        sqlx::query(
            "INSERT INTO artifact (id, message_id, filename, html, sandboxed, created_at) \
                 VALUES ('art-1', 'msg-1', 'page.html', '<h1>hi</h1>', 1, '2020-01-01T00:00:00+00:00')",
        )
        .execute(&pool)
        .await
        .expect("seed pre-0014 artifact");

        // Apply migrate() from v13; the 0014 rebuild remains the behavior
        // under test even though later additive migrations may also fire.
        migrate(&pool).await.expect("0014 migration failed");

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("user_version query failed");
        assert_eq!(version, 23, "migrate() from v13 must reach schema v23");

        // The legacy row survived, folded into the new shape.
        let row = crate::engine::repo::artifact::get_artifact(&pool, "art-1")
            .await
            .expect("get failed")
            .expect("legacy artifact must survive the rebuild");
        assert_eq!(
            row.message_id.as_deref(),
            Some("msg-1"),
            "message_id preserved"
        );
        assert_eq!(
            row.filename.as_deref(),
            Some("page.html"),
            "filename preserved"
        );
        assert_eq!(
            row.content.as_deref(),
            Some("<h1>hi</h1>"),
            "html folded into content"
        );
        assert_eq!(
            row.kind.as_deref(),
            Some("html"),
            "legacy rows tagged kind='html'"
        );
        assert_eq!(row.sandboxed, Some(true), "sandboxed 1 preserved (as bool)");
        assert!(
            row.workspace_id.is_none(),
            "legacy rows were never workspace-scoped"
        );

        // Wire shape of the migrated legacy row (Armin, challenge d66edb10):
        // sandboxed is a JSON bool, messageId is present, the null
        // workspace/agent columns are omitted rather than serialised as null.
        let json = serde_json::to_value(&row).expect("serialize failed");
        assert_eq!(
            json.get("sandboxed"),
            Some(&serde_json::Value::Bool(true)),
            "sandboxed must serialise as a JSON bool, not the integer 1"
        );
        assert_eq!(
            json.get("messageId").and_then(|v| v.as_str()),
            Some("msg-1")
        );
        assert_eq!(json.get("kind").and_then(|v| v.as_str()), Some("html"));
        assert!(
            json.get("workspaceId").is_none(),
            "null workspaceId omitted"
        );
        assert!(json.get("agentId").is_none(), "null agentId omitted");

        // No foreign key left dangling by the rebuild.
        let fk_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .expect("foreign_key_check query failed");
        assert!(
            fk_violations.is_empty(),
            "0014 left dangling FKs: {fk_violations:?}"
        );
    }

    /// Build an in-memory DB at schema **v17** — the state just before 0018 —
    /// mirroring [`connect_at_v13`]: apply 0001..0017 exactly as [`migrate`]
    /// would, then stop. Lets a test exercise the REAL 0018 `task_event`
    /// rebuild against populated v17 data, which fresh-schema tests cannot.
    async fn connect_at_v17() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .expect("invalid in-memory connection string")
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("failed to open in-memory pool");
        let mut tx = pool.begin().await.expect("begin v17 setup");
        for sql in [
            include_str!("migrations/0001_init.sql"),
            include_str!("migrations/0002_seed_core_tools.sql"),
            include_str!("migrations/0003_agent_cli_config.sql"),
            include_str!("migrations/0004_skill_system.sql"),
            include_str!("migrations/0005_drop_skill_kind.sql"),
            include_str!("migrations/0006_selected_builtin_skills.sql"),
            include_str!("migrations/0007_workspace_hidden.sql"),
            include_str!("migrations/0008_role_system.sql"),
            include_str!("migrations/0009_memory_system.sql"),
            include_str!("migrations/0010_memory_search_index.sql"),
            include_str!("migrations/0011_seed_memory_tool.sql"),
            include_str!("migrations/0012_task_system.sql"),
            include_str!("migrations/0013_memory_proposal.sql"),
            include_str!("migrations/0014_artifact_workspace.sql"),
            include_str!("migrations/0015_position_system.sql"),
            include_str!("migrations/0016_agent_default_level.sql"),
            include_str!("migrations/0017_agent_rtk_enabled.sql"),
        ] {
            sqlx::raw_sql(sql)
                .execute(&mut *tx)
                .await
                .expect("apply pre-0018 migration");
        }
        sqlx::raw_sql("PRAGMA user_version = 17;")
            .execute(&mut *tx)
            .await
            .expect("set user_version = 17");
        tx.commit().await.expect("commit v17 setup");
        pool
    }

    /// The 0018 upgrade path itself (ruling on challenge 15d636ab): the
    /// `task_event` rebuild must preserve every existing row byte-for-byte
    /// and keep `idx_task_event_task`, while admitting the new `plan_check`
    /// kind and still rejecting unknown kinds. Fresh-schema tests exercise
    /// only the post-0018 shape and could not catch a lossy copy or a
    /// dropped index.
    #[tokio::test]
    async fn migrate_0018_preserves_task_event_rows_and_index() {
        let pool = connect_at_v17().await;

        // Seed a v17 task + one event per legacy kind (task_event has no FK,
        // and the task row itself is untouched by 0018).
        sqlx::query(
            "INSERT INTO task (id, workspace_id, slug, title, owner_agent_id, \
             created_at, updated_at) \
             VALUES ('task-v17', 'ws-v17', 't1', 'T1', 'agent-1', \
             '2020-01-01T00:00:00+00:00', '2020-01-01T00:00:00+00:00')",
        )
        .execute(&pool)
        .await
        .expect("seed v17 task");
        for (n, kind) in ["note", "state", "gate", "challenge", "ruling"]
            .iter()
            .enumerate()
        {
            sqlx::query(
                "INSERT INTO task_event (id, task_id, kind, actor_agent_id, payload, created_at) \
                 VALUES (?1, 'task-v17', ?2, 'agent-1', ?3, ?4)",
            )
            .bind(format!("ev-{kind}"))
            .bind(kind)
            .bind(format!("{{\"n\":{n}}}"))
            .bind(format!("2020-01-01T00:00:0{n}+00:00"))
            .execute(&pool)
            .await
            .expect("seed v17 task_event");
        }

        migrate(&pool).await.expect("0018 migration failed");

        // Every legacy row survived with payload/actor/timestamps intact.
        let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, kind, actor_agent_id, payload, created_at \
             FROM task_event WHERE task_id = 'task-v17' ORDER BY created_at",
        )
        .fetch_all(&pool)
        .await
        .expect("read migrated task_event rows");
        assert_eq!(rows.len(), 5, "all five legacy rows must survive");
        for (n, kind) in ["note", "state", "gate", "challenge", "ruling"]
            .iter()
            .enumerate()
        {
            let (id, k, actor, payload, created_at) = &rows[n];
            assert_eq!(id, &format!("ev-{kind}"));
            assert_eq!(k, kind);
            assert_eq!(actor, "agent-1");
            assert_eq!(payload, &format!("{{\"n\":{n}}}"));
            assert_eq!(created_at, &format!("2020-01-01T00:00:0{n}+00:00"));
        }

        // The covering index survived the DROP/RENAME rebuild.
        let index_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master \
             WHERE type='index' AND name='idx_task_event_task' AND tbl_name='task_event'",
        )
        .fetch_one(&pool)
        .await
        .expect("index existence query failed");
        assert_eq!(index_count, 1, "idx_task_event_task must survive 0018");

        // Old kinds still insert, the new plan_check kind now inserts, and an
        // unknown kind is still rejected by the rebuilt CHECK.
        for kind in ["note", "plan_check"] {
            sqlx::query(
                "INSERT INTO task_event (id, task_id, kind, payload, created_at) \
                 VALUES (?1, 'task-v17', ?2, '{}', '2020-01-02T00:00:00+00:00')",
            )
            .bind(format!("ev-new-{kind}"))
            .bind(kind)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("kind `{kind}` must insert post-0018: {e}"));
        }
        let bogus = sqlx::query(
            "INSERT INTO task_event (id, task_id, kind, payload, created_at) \
             VALUES ('ev-bogus', 'task-v17', 'bogus', '{}', '2020-01-02T00:00:00+00:00')",
        )
        .execute(&pool)
        .await;
        assert!(bogus.is_err(), "unknown kinds must still violate the CHECK");
    }

    /// All entity tables must exist after migration.
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
        assert_eq!(count, 28, "expected 28 tables, got {count}");
    }

    /// Running migrate twice must not error and must leave user_version == 19.
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
            count, 28,
            "expected 28 tables after idempotent run, got {count}"
        );

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("user_version query failed");
        assert_eq!(version, 23, "user_version should be 23");

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

    /// Structural guard (global constraint 9, "Ordering rule" — added after a
    /// migration-sequencing near-miss flagged during memory-v1 T5): migration
    /// files must be numbered contiguously from 0001 with NO gaps, and a
    /// fresh `migrate()` must land `user_version` at exactly the max file
    /// number. A skipped-version state (e.g. a migration merged before its
    /// predecessor's `if version < N` block landed in `db.rs`) fails THIS
    /// gate instead of silently shipping — see the ordering-rule doc above.
    #[tokio::test]
    async fn migration_files_are_contiguous_and_migrate_reaches_the_max() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/engine/migrations");
        let mut numbers: Vec<u32> = std::fs::read_dir(&dir)
            .expect("read migrations dir")
            .map(|entry| {
                entry
                    .expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter_map(|name| name.get(0..4).and_then(|prefix| prefix.parse::<u32>().ok()))
            .collect();
        numbers.sort_unstable();
        numbers.dedup();

        assert!(!numbers.is_empty(), "no migration files found in {dir:?}");
        assert_eq!(numbers[0], 1, "migrations must start at 0001");
        for pair in numbers.windows(2) {
            assert_eq!(
                pair[1],
                pair[0] + 1,
                "migration numbering has a gap between {:04} and {:04} — a merged \
                 migration file with no db.rs version<N block, or vice versa",
                pair[0],
                pair[1]
            );
        }

        let max_number = *numbers.last().expect("checked non-empty above");
        let pool = connect_in_memory().await;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("user_version query failed");
        assert_eq!(
            version,
            i64::from(max_number),
            "a fresh migrate() must land user_version at the highest migration file number"
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

    /// Migration 0011 seeds exactly one `tool-memory` core row (independently
    /// toggleable per agent, unlike `tool-conclave`'s whole-cli-surface row);
    /// running migrate again leaves exactly one row.
    #[tokio::test]
    async fn migrate_seeds_core_memory_tool() {
        let pool = connect_in_memory().await;

        let (id, is_core, kind): (String, i64, String) =
            sqlx::query_as("SELECT id, is_core, kind FROM tool WHERE id = 'tool-memory'")
                .fetch_one(&pool)
                .await
                .expect("memory tool row should exist after migration");

        assert_eq!(id, "tool-memory");
        assert_eq!(is_core, 1, "is_core must be 1");
        assert_eq!(kind, "builtin");

        migrate(&pool)
            .await
            .expect("second migrate must be a no-op");

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tool WHERE id = 'tool-memory'")
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
        assert_eq!(version, 23);
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
        assert_eq!(version, 23);
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
        let skill_ids: String = sqlx::query_scalar("SELECT skill_ids FROM role WHERE id = 'r1'")
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

    /// Migration 0009 creates the workspace-scoped memory index and chunk
    /// tables, including the idempotency uniqueness constraint.
    #[tokio::test]
    async fn migrate_adds_memory_system_tables() {
        let pool = connect_in_memory().await;

        sqlx::query(
            "INSERT INTO workspace (id, name, folder_path, created_at) \
             VALUES ('memory-ws', 'Memory', '/tmp/memory', '2026-07-04T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert workspace");
        sqlx::query(
            "INSERT INTO memory_index \
             (workspace_id, model_id, dimension, created_at, updated_at) \
             VALUES ('memory-ws', 'fake', 2, '2026-07-04T00:00:00Z', \
                     '2026-07-04T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert memory index");
        sqlx::query(
            "INSERT INTO memory_chunk \
             (id, workspace_id, source_kind, text, embedding, dimension, content_hash, \
              created_at, updated_at) \
             VALUES ('chunk-1', 'memory-ws', 'manual', 'text', x'0000000000000000', 2, \
                     'hash', '2026-07-04T00:00:00Z', '2026-07-04T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert memory chunk");

        let duplicate = sqlx::query(
            "INSERT INTO memory_chunk \
             (id, workspace_id, source_kind, text, embedding, dimension, content_hash, \
              created_at, updated_at) \
             VALUES ('chunk-2', 'memory-ws', 'manual', 'text', x'0000000000000000', 2, \
                     'hash', '2026-07-04T00:00:00Z', '2026-07-04T00:00:00Z')",
        )
        .execute(&pool)
        .await;
        assert!(
            duplicate.is_err(),
            "workspace/content hash uniqueness must reject duplicates"
        );

        sqlx::query("DELETE FROM workspace WHERE id = 'memory-ws'")
            .execute(&pool)
            .await
            .expect("delete workspace");
        let remaining_index: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM memory_index WHERE workspace_id = 'memory-ws'",
        )
        .fetch_one(&pool)
        .await
        .expect("count memory index");
        let remaining_chunks: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM memory_chunk WHERE workspace_id = 'memory-ws'",
        )
        .fetch_one(&pool)
        .await
        .expect("count memory chunks");
        assert_eq!(remaining_index, 0, "memory index must cascade");
        assert_eq!(remaining_chunks, 0, "memory chunks must cascade");

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("pragma read failed");
        assert_eq!(version, 23);
    }

    /// Migration 0010 adds the composite index required for workspace-scoped
    /// keyset scans without a per-page temporary B-tree sort.
    #[tokio::test]
    async fn migrate_adds_memory_search_index() {
        let pool = connect_in_memory().await;

        let index_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master \
             WHERE type = 'index' AND name = 'idx_memory_chunk_ws_id'",
        )
        .fetch_one(&pool)
        .await
        .expect("index lookup");
        assert_eq!(index_count, 1, "memory search index must exist");

        let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
            "EXPLAIN QUERY PLAN \
             SELECT id, embedding, dimension FROM memory_chunk \
             WHERE workspace_id = ?1 AND id > ?2 ORDER BY id ASC LIMIT 512",
        )
        .bind("workspace")
        .bind("cursor")
        .fetch_all(&pool)
        .await
        .expect("query plan");
        let details: Vec<&str> = plan.iter().map(|row| row.3.as_str()).collect();
        assert!(
            details
                .iter()
                .any(|detail| detail.contains("idx_memory_chunk_ws_id")),
            "query must use the composite index: {details:?}"
        );
        assert!(
            details.iter().all(|detail| !detail.contains("TEMP B-TREE")),
            "query must not sort each page into a temp B-tree: {details:?}"
        );
    }
}

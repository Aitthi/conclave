//! Database connection management and migration runner.
//!
//! Entry points:
//! - [`open`]           — on-disk connection for production use
//! - [`open_in_memory`] — in-memory connection for tests
//!
//! Migration strategy: `PRAGMA user_version` is the version counter.
//! Each milestone appends a new version gate inside [`migrate`].

use std::path::PathBuf;

use rusqlite::{Connection, Result};

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

/// Opens the on-disk SQLite database, applies PRAGMAs, and runs pending migrations.
///
/// # Errors
/// Returns [`rusqlite::Error`] if the database file cannot be opened or migrated.
/// The caller (startup) is expected to `.expect()` on this.
pub fn open() -> Result<Connection> {
    let conn = Connection::open(db_path())?;
    apply_pragmas(&conn, true)?;
    migrate(&conn)?;
    Ok(conn)
}

/// Opens an in-memory SQLite connection suitable for unit tests.
/// Applies the same PRAGMAs as [`open`] (WAL is skipped — incompatible with
/// `:memory:`) and runs all migrations.
///
/// # Panics
/// Panics if the in-memory database cannot be opened or migrated.
#[cfg(test)]
pub fn open_in_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("failed to open in-memory db");
    apply_pragmas(&conn, false).expect("failed to apply pragmas to in-memory db");
    migrate(&conn).expect("failed to migrate in-memory db");
    conn
}

/// Applies standard PRAGMAs to `conn`.
///
/// - `wal`: also sets `journal_mode=WAL`; pass `false` for in-memory databases.
fn apply_pragmas(conn: &Connection, wal: bool) -> Result<()> {
    if wal {
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    }
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    Ok(())
}

/// Idempotent migration runner.
///
/// Reads `PRAGMA user_version` and applies pending migrations in order.
/// Each migration runs inside an explicit `BEGIN … COMMIT` block, and the
/// `user_version` bump is issued INSIDE that transaction (SQLite makes
/// `user_version` transactional). This makes apply-and-bump atomic: a crash
/// mid-migration rolls back the DDL *and* the version, so the schema and the
/// version counter can never disagree — re-running on next launch is safe.
///
/// **Adding future migrations**: append another `if user_version < N { … }` block.
fn migrate(conn: &Connection) -> Result<()> {
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if user_version < 1 {
        conn.execute_batch(concat!(
            "BEGIN;\n",
            include_str!("migrations/0001_init.sql"),
            "\nPRAGMA user_version = 1;\n",
            "COMMIT;"
        ))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 19 entity tables must exist after migration.
    #[test]
    fn migrate_creates_all_tables() {
        let conn = open_in_memory();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master \
                 WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .expect("table-count query failed");
        assert_eq!(count, 19, "expected 19 tables, got {count}");
    }

    /// Running migrate twice must not error and must leave user_version == 1.
    #[test]
    fn migrate_is_idempotent() {
        let conn = Connection::open_in_memory().expect("open failed");
        apply_pragmas(&conn, false).expect("pragmas failed");

        migrate(&conn).expect("first migration failed");
        migrate(&conn).expect("second migration must be a no-op");

        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master \
                 WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .expect("table-count query failed");
        assert_eq!(
            count, 19,
            "expected 19 tables after idempotent run, got {count}"
        );

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version query failed");
        assert_eq!(version, 1, "user_version should be 1");
    }

    /// With `PRAGMA foreign_keys=ON`, inserting a `workspace_agent` row that
    /// references a non-existent workspace must fail.
    #[test]
    fn foreign_keys_enforced() {
        let conn = open_in_memory();
        let result = conn.execute(
            "INSERT INTO workspace_agent(id, workspace_id, agent_def_id, status, added_at) \
             VALUES ('wa-1', 'no-such-workspace', 'no-such-agent', 'idle', '2024-01-01T00:00:00Z')",
            [],
        );
        assert!(
            result.is_err(),
            "FK violation against workspace should return an error"
        );
    }
}

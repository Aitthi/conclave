//! Central application state threaded through every IPC handler.
//!
//! `AppState::new()` opens the SQLite connection pool and runs pending
//! migrations at startup. It will panic (via `.expect`) if the database
//! cannot be initialised — this is intentional: a missing or corrupt
//! database is not a recoverable situation at launch time.

use sqlx::SqlitePool;

pub struct AppState {
    /// Live, migration-applied SQLite connection pool.
    ///
    /// `SqlitePool` is `Clone + Send + Sync`, so no `Mutex` is needed.
    /// Handlers that need the DB can use `&state.db` directly.
    ///
    /// `#[allow(dead_code)]`: no command handler reads `db` yet (M1+).
    #[allow(dead_code)]
    pub db: SqlitePool,
}

impl AppState {
    /// Open the on-disk database pool, apply pending migrations, and return
    /// an initialised `AppState`.
    ///
    /// # Panics
    /// Panics if the database file cannot be opened or any migration fails.
    pub fn new() -> Self {
        let pool = tauri::async_runtime::block_on(crate::engine::db::connect())
            .expect("failed to open/migrate Conclave database");
        Self { db: pool }
    }
}

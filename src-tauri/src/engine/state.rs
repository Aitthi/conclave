//! Central application state threaded through every IPC handler.
//!
//! `AppState::new()` opens the SQLite database and runs pending migrations at
//! startup. It will panic (via `.expect`) if the database cannot be
//! initialised — this is intentional: a missing or corrupt database is not a
//! recoverable situation at launch time.

use rusqlite::Connection;

pub struct AppState {
    /// Live, migration-applied SQLite connection.
    ///
    /// Wrapped in a `tokio::sync::Mutex` so it is `Send + Sync` and can be
    /// held across `.await` points inside async Tauri command handlers.
    /// Handlers that need the DB: `let db = state.db.lock().await;`.
    ///
    /// `#[allow(dead_code)]`: no command handler reads `db` yet (M0.4+).
    #[allow(dead_code)]
    pub db: tokio::sync::Mutex<Connection>,
}

impl AppState {
    /// Open the on-disk database, apply pending migrations, and return an
    /// initialised `AppState`.
    ///
    /// # Panics
    /// Panics if the database file cannot be opened or any migration fails.
    pub fn new() -> Self {
        let conn = crate::engine::db::open().expect("failed to open/migrate Conclave database");
        Self {
            db: tokio::sync::Mutex::new(conn),
        }
    }
}

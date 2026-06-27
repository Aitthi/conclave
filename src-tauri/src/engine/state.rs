//! Central application state threaded through every IPC handler.
//!
//! `AppState::new()` opens the SQLite connection pool and runs pending
//! migrations at startup. It will panic (via `.expect`) if the database
//! cannot be initialised — this is intentional: a missing or corrupt
//! database is not a recoverable situation at launch time.

use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::OnceLock;
use tauri::AppHandle;

pub struct AppState {
    /// Live, migration-applied SQLite connection pool.
    ///
    /// `SqlitePool` is `Clone + Send + Sync`, so no `Mutex` is needed.
    /// Handlers that need the DB pass `&state.db` to repo functions.
    pub db: SqlitePool,

    /// Tauri application handle, stored once during `.setup()`.
    ///
    /// `OnceLock<AppHandle>` is `Send + Sync` because `AppHandle` is
    /// `Send + Sync + Clone`, satisfying the bounds that `tauri::State`
    /// requires of managed state.
    app: OnceLock<AppHandle>,
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
        Self {
            db: pool,
            app: OnceLock::new(),
        }
    }

    /// Store the `AppHandle` obtained during Tauri's `.setup()` phase.
    ///
    /// Silently ignores a second call — the handle is intended to be set
    /// exactly once and never replaced.
    pub fn set_app(&self, handle: AppHandle) {
        if self.app.set(handle).is_err() {
            #[cfg(debug_assertions)]
            eprintln!("[bus] set_app called more than once — second handle dropped");
        }
    }

    /// Return a reference to the stored `AppHandle`, or `None` if `.setup()`
    /// has not yet completed.
    ///
    /// `#[allow(dead_code)]`: consumed by the runtime in M2.
    #[allow(dead_code)]
    pub fn app(&self) -> Option<&AppHandle> {
        self.app.get()
    }

    /// Emit a typed event to the Tauri frontend.
    ///
    /// Non-fatal: if the `AppHandle` has not been set yet, a debug-only
    /// message is printed and the call returns silently.  This lets handler
    /// code call `state.emit(...)` without needing to propagate `Option`
    /// or `Result` just for the handle-not-ready case.
    ///
    /// `#[allow(dead_code)]`: the runtime begins emitting in M2.
    #[allow(dead_code)]
    pub fn emit<T: Serialize + Clone>(&self, event: &str, payload: T) {
        match self.app.get() {
            Some(handle) => {
                if let Err(e) = super::bus::emit(handle, event, payload) {
                    eprintln!("[bus] emit error on event \"{event}\": {e}");
                }
            }
            None => {
                #[cfg(debug_assertions)]
                eprintln!("[bus] emit(\"{event}\") called before AppHandle was set");
            }
        }
    }
}

/// Test-only constructor: builds an `AppState` backed by an in-memory
/// SQLite pool (migration applied). The `AppHandle` is intentionally absent
/// — tests that do not emit events work fine without it.
#[cfg(test)]
impl AppState {
    pub(crate) async fn for_tests() -> Self {
        Self {
            db: crate::engine::db::connect_in_memory().await,
            app: OnceLock::new(),
        }
    }
}

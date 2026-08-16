//! Central application state threaded through every IPC handler.
//!
//! `AppState::new()` opens the SQLite connection pool and runs pending
//! migrations at startup. It will panic (via `.expect`) if the database
//! cannot be initialised — this is intentional: a missing or corrupt
//! database is not a recoverable situation at launch time.

use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::AppHandle;

/// How long a restart stays armed. A never-saving agent must not leave a stale
/// arm that later hijacks an unrelated manual `snapshot save` into a restart.
const RESTART_PENDING_TTL: Duration = Duration::from_secs(300);

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

    /// In-memory registry of live agent sessions (M2 runtime core).
    ///
    /// A pure concurrency primitive with no DB/Tauri dependency; status
    /// persistence and event emission happen in the command handlers.
    ///
    /// `Arc` so a PTY backend's detached forwarder task can hold its own
    /// reference and clean up the registry when the child self-terminates
    /// (M2.2), independent of the `AppState`'s lifetime.
    pub runtime: std::sync::Arc<crate::engine::runtime::Runtime>,

    /// Text embedder for the workspace memory system (memory-v1 T3).
    ///
    /// Object-safe (`Arc<dyn Embedder>`) so command handlers (T4) are
    /// generic over the backend: production wiring holds a
    /// `FastembedEmbedder`, `AppState::for_tests` holds a `FakeEmbedder` so
    /// tests never depend on the model download gate. Model load is lazy
    /// inside `FastembedEmbedder` itself — constructing this field touches
    /// neither the filesystem nor the network.
    ///
    pub memory_embedder: std::sync::Arc<dyn crate::engine::runtime::embedder::Embedder>,

    /// Decoded memory vectors for warm exact search. The cache is bounded to
    /// four workspaces and invalidated by every completed memory write.
    pub memory_search_cache: std::sync::Arc<crate::engine::commands::memory::MemorySearchCache>,

    /// Per-root incremental tree-sitter index cache shared by all `code.*`
    /// handlers.
    pub code_cache: std::sync::Arc<codeintel::cache::CodeIntelCache>,

    /// Instances with a restart ARMED: the agent's next `conclave snapshot save`
    /// triggers kill → respawn → resume (see `commands::instance::restart`).
    /// Keyed by instance id → arm time, with consume-once + TTL discipline so
    /// a stale arm cannot hijack a later unrelated save.
    restart_pending: Mutex<HashMap<String, Instant>>,
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
            runtime: std::sync::Arc::new(crate::engine::runtime::Runtime::new()),
            memory_embedder: std::sync::Arc::new(
                crate::engine::runtime::embedder::FastembedEmbedder::with_default_cache_dir(),
            ),
            memory_search_cache: std::sync::Arc::new(
                crate::engine::commands::memory::MemorySearchCache::new(),
            ),
            code_cache: std::sync::Arc::new(codeintel::cache::CodeIntelCache::new()),
            restart_pending: Mutex::new(HashMap::new()),
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

    /// Arm a restart for `instance_id`: the agent's next handoff save triggers
    /// kill → respawn → resume (see `commands::instance::restart`). Overwrites
    /// any prior arm.
    pub fn mark_restart_pending(&self, instance_id: &str) {
        if let Ok(mut m) = self.restart_pending.lock() {
            m.insert(instance_id.to_owned(), Instant::now());
        }
    }

    /// Consume a pending restart for `instance_id` (consume-once, TTL-guarded).
    pub fn take_restart_pending(&self, instance_id: &str) -> bool {
        if let Ok(mut m) = self.restart_pending.lock() {
            if let Some(armed) = m.remove(instance_id) {
                return armed.elapsed() < RESTART_PENDING_TTL;
            }
        }
        false
    }

    /// The TTL a restart arm stays valid for. Exposed so a self-triggered
    /// restart's returned instruction (ADR 0006) can surface the SAME
    /// deadline `take_restart_pending` actually holds it to, rather than a
    /// hand-copied literal that could silently drift from the real constant.
    pub fn restart_pending_ttl(&self) -> Duration {
        RESTART_PENDING_TTL
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
            runtime: std::sync::Arc::new(crate::engine::runtime::Runtime::new()),
            memory_embedder: std::sync::Arc::new(
                crate::engine::runtime::embedder::FakeEmbedder::new(
                    crate::engine::runtime::embedder::MINILM_L6_V2_Q_DIMENSION,
                ),
            ),
            memory_search_cache: std::sync::Arc::new(
                crate::engine::commands::memory::MemorySearchCache::new(),
            ),
            code_cache: std::sync::Arc::new(codeintel::cache::CodeIntelCache::new()),
            restart_pending: Mutex::new(HashMap::new()),
        }
    }
}

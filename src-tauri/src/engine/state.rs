/// Central application state threaded through every IPC handler.
///
/// M0.3 will replace the `db` placeholder with a real rusqlite Connection
/// or connection-pool once the migration layer is ready.
pub struct AppState {
    /// Placeholder for the database connection.
    // M0.3 will replace `Option<()>` with a rusqlite Connection / pool.
    #[allow(dead_code)]
    pub db: tokio::sync::Mutex<Option<()>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            db: tokio::sync::Mutex::new(None),
        }
    }
}

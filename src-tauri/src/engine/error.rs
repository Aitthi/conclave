use thiserror::Error;

/// Central error type for all IPC dispatch paths.
///
/// The Tauri boundary maps this to a plain `String` via `Display`
/// (`.map_err(|e| e.to_string())` in the `ipc` command).
///
/// The `From<sqlx::Error>` impl lets repo calls be `?`-chained in handlers:
/// `repo::workspace::list(&state.db).await?` automatically maps to `Internal`
/// (or `NotFound` for `RowNotFound`).
#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid: {0}")]
    Invalid(String),

    #[error("internal: {0}")]
    Internal(String),

    // Constructed by future milestone handlers; suppress until then.
    #[allow(dead_code)]
    #[error("not implemented: {0}")]
    NotImplemented(String),
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::RowNotFound => AppError::NotFound("record not found".into()),
            other => AppError::Internal(other.to_string()),
        }
    }
}

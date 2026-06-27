use thiserror::Error;

/// Central error type for all IPC dispatch paths.
/// The Tauri boundary maps this to a plain `String` via `Display`
/// (`.map_err(|e| e.to_string())` in the `ipc` command).
///
/// `Internal` / `NotImplemented` are part of the intended error taxonomy;
/// they are constructed once real handlers land in M0.3+ — allow until then.
#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid: {0}")]
    Invalid(String),

    #[error("internal: {0}")]
    Internal(String),

    #[error("not implemented: {0}")]
    NotImplemented(String),
}

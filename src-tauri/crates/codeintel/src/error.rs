use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Errors returned by library-level helpers in `codegraph-core`. Binaries
/// (`codegraph`, `astedit`) wrap these in their own error enums to enrich
/// with command-specific context.
///
/// `#[non_exhaustive]` so variants can be added without a major-version bump.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CoreError {
    /// Filesystem read/stat failed for `path`.
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

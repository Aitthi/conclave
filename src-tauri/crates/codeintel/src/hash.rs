use std::fmt;

/// SHA-256 of a file's contents. Wraps a raw 32-byte array so callers get
/// hex `Display`, structural equality, and a clear type name in `Debug`
/// output instead of an opaque byte array.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileHash([u8; 32]);

impl FileHash {
    /// Wrap a 32-byte digest.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return the underlying digest bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for FileHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileHash({self})")
    }
}

impl fmt::Display for FileHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl AsRef<[u8]> for FileHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::CoreError;

/// Compute the SHA-256 of a file's contents. Streams through a 64 KiB
/// buffer so memory use stays bounded regardless of file size.
///
/// Returns `CoreError::Io` wrapping the path on any filesystem error.
pub fn compute_file_hash(path: &Path) -> Result<FileHash, CoreError> {
    let file = File::open(path).map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let bytes: [u8; 32] = hasher.finalize().into();
    Ok(FileHash::new(bytes))
}

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crate::hash::compute_file_hash;
use crate::index::FileMeta;

use crate::edit::error::AstEditError;

/// Atomic per-file write. Writes `bytes` to a temp file in the same directory
/// as `target`, then `rename(2)`s it over `target`. Same-directory placement
/// keeps `rename` atomic on every supported filesystem.
///
/// Returns `WriteFailed` on any IO error, with `os_code` populated from
/// `io::Error::raw_os_error()`.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), AstEditError> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("astedit.tmp");
    let tmp_path = dir.join(format!(".{file_name}.astedit.tmp"));
    let map_err = |e: std::io::Error| AstEditError::WriteFailed {
        file: target.to_string_lossy().into_owned(),
        os_code: e.raw_os_error(),
        message: e.to_string(),
    };

    {
        let mut f = File::create(&tmp_path).map_err(map_err)?;
        f.write_all(bytes).map_err(map_err)?;
        f.sync_all().map_err(map_err)?;
    }
    fs::rename(&tmp_path, target).map_err(|e| {
        // Best-effort cleanup; ignore failure since the original is still intact.
        let _ = fs::remove_file(&tmp_path);
        AstEditError::WriteFailed {
            file: target.to_string_lossy().into_owned(),
            os_code: e.raw_os_error(),
            message: e.to_string(),
        }
    })?;
    Ok(())
}

/// Detect whether `target` has drifted from the snapshot recorded in
/// `Index.file_meta` at index time.
///
/// Returns `Ok(())` if the file is unchanged (length match, OR length differs
/// but the on-demand SHA matches the supplied `index_hash`).
/// Returns `Err(HashMismatch)` if drift is detected (length mismatch AND
/// either no `index_hash` was supplied or the recomputed hash differs).
/// Returns `Err(WriteFailed)` on IO failure during the stat call.
///
/// `index_hash` is `None` when the index didn't hash the file eagerly
/// (it never does in PR 1's `build_index`). When `None` AND lengths
/// differ, drift is assumed — there's no cheap way to confirm.
pub fn check_drift(
    target: &Path,
    rel_path: &str,
    meta: &FileMeta,
    index_hash: Option<&crate::hash::FileHash>,
) -> Result<(), AstEditError> {
    let stat = fs::metadata(target).map_err(|e| AstEditError::WriteFailed {
        file: rel_path.to_string(),
        os_code: e.raw_os_error(),
        message: e.to_string(),
    })?;
    if stat.len() == meta.len {
        return Ok(());
    }

    // Length mismatch — fall back to SHA-256. `CoreError` is defined in this
    // same crate (unlike the original astedit port, where it came from the
    // separate codegraph-core crate), so matching its one variant is already
    // exhaustive — no catch-all arm needed.
    let on_disk = compute_file_hash(target).map_err(|e| match e {
        crate::CoreError::Io { source, .. } => AstEditError::WriteFailed {
            file: rel_path.to_string(),
            os_code: source.raw_os_error(),
            message: source.to_string(),
        },
    })?;
    match index_hash {
        Some(h) if h == &on_disk => Ok(()),
        _ => Err(AstEditError::HashMismatch {
            file: rel_path.to_string(),
        }),
    }
}

/// Stat `target` and return its current length. Used by the race-window
/// guard right before a write — compared against the length read at step 5a.
pub fn current_len(target: &Path, rel_path: &str) -> Result<u64, AstEditError> {
    fs::metadata(target)
        .map(|m| m.len())
        .map_err(|e| AstEditError::WriteFailed {
            file: rel_path.to_string(),
            os_code: e.raw_os_error(),
            message: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn write_atomic_replaces_existing_content() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("x.txt");
        fs::write(&target, b"old").unwrap();

        write_atomic(&target, b"new content").unwrap();

        let mut s = String::new();
        File::open(&target).unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "new content");
    }

    #[test]
    fn write_atomic_leaves_no_temp_files() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("y.txt");
        fs::write(&target, b"old").unwrap();
        write_atomic(&target, b"new").unwrap();

        let leftovers: Vec<PathBuf> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                let n = p.file_name().unwrap().to_string_lossy().to_string();
                if n.ends_with(".astedit.tmp") {
                    Some(p)
                } else {
                    None
                }
            })
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    }

    #[test]
    fn check_drift_ok_when_length_matches() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("z.txt");
        fs::write(&target, b"hello").unwrap();

        let mut meta = FileMeta::default();
        meta.len = 5;
        check_drift(&target, "z.txt", &meta, None).unwrap();
    }

    #[test]
    fn check_drift_errors_when_length_differs_and_no_hash() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("z.txt");
        fs::write(&target, b"hello world").unwrap();

        let mut meta = FileMeta::default();
        meta.len = 5;
        let err = check_drift(&target, "z.txt", &meta, None).unwrap_err();
        assert_eq!(err.kind(), "hash-mismatch");
    }
}

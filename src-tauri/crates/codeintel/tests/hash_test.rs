use codeintel::hash::FileHash;

#[test]
fn file_hash_display_is_lowercase_hex() {
    let bytes = [0xab_u8; 32];
    let hash = FileHash::new(bytes);
    let s = format!("{hash}");
    assert_eq!(s.len(), 64);
    assert!(s
        .chars()
        .all(|c| c.is_ascii_hexdigit() && (!c.is_alphabetic() || c.is_lowercase())));
    assert_eq!(s, "ab".repeat(32));
}

#[test]
fn file_hash_as_ref_returns_underlying_bytes() {
    let bytes = [0x42_u8; 32];
    let hash = FileHash::new(bytes);
    let slice: &[u8] = hash.as_ref();
    assert_eq!(slice, &bytes[..]);
}

#[test]
fn file_hash_equality() {
    assert_eq!(FileHash::new([0; 32]), FileHash::new([0; 32]));
    assert_ne!(FileHash::new([0; 32]), FileHash::new([1; 32]));
}

use codeintel::hash::compute_file_hash;
use std::fs;
use tempfile::TempDir;

#[test]
fn compute_file_hash_matches_known_sha256() {
    // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("data.txt");
    fs::write(&path, b"abc").unwrap();

    let hash = compute_file_hash(&path).expect("hash");
    assert_eq!(
        format!("{hash}"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
}

#[test]
fn compute_file_hash_streams_large_input() {
    // Verify the streaming loop produces the same digest as a single-pass
    // call on input larger than the 64 KiB buffer.
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("big.bin");
    let body = vec![0x5a_u8; 200 * 1024]; // 200 KiB, well over the buffer
    fs::write(&path, &body).unwrap();

    let hash = compute_file_hash(&path).expect("hash");

    // Reference digest computed independently.
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&body);
    let expected: [u8; 32] = h.finalize().into();
    assert_eq!(hash.as_bytes(), &expected);
}

#[test]
fn compute_file_hash_missing_file_returns_io_error() {
    use codeintel::CoreError;
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("not-there.bin");

    let err = compute_file_hash(&missing).expect_err("should error");
    match err {
        CoreError::Io { ref path, .. } => assert_eq!(path, &missing),
        _ => panic!("expected Io error"),
    }
}

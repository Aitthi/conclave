use codeintel::index::build_index;
use std::fs;
use tempfile::TempDir;

#[test]
fn build_index_populates_file_meta_len() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("hello.rs");
    let body = "fn hello() {}\n";
    fs::write(&path, body).unwrap();

    let index = build_index(tmp.path()).unwrap();

    let key = "hello.rs"; // repo-relative, forward-slash-normalized
    let meta = index
        .file_meta
        .get(key)
        .expect("file_meta should contain hello.rs");
    assert_eq!(meta.len, body.len() as u64);
}

use codeintel::cache::CodeIntelCache;
use std::fs;

#[test]
fn warm_hit_returns_same_arc_when_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lib.rs"), "fn a() {}\n").unwrap();
    let cache = CodeIntelCache::new();
    let one = cache.get_index(dir.path()).unwrap();
    let two = cache.get_index(dir.path()).unwrap();
    assert!(std::sync::Arc::ptr_eq(&one, &two));
}

#[test]
fn edited_file_is_reindexed() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("lib.rs");
    fs::write(&f, "fn a() {}\n").unwrap();
    let cache = CodeIntelCache::new();
    assert!(cache.get_index(dir.path()).unwrap().definitions.iter().any(|d| d.name == "a"));
    fs::write(&f, "fn b() {}\n").unwrap();
    let idx = cache.get_index(dir.path()).unwrap();
    assert!(idx.definitions.iter().any(|d| d.name == "b"));
    assert!(!idx.definitions.iter().any(|d| d.name == "a"));
}

#[test]
fn deleted_file_leaves_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("lib.rs");
    fs::write(&f, "fn a() {}\n").unwrap();
    fs::write(dir.path().join("keep.rs"), "fn k() {}\n").unwrap();
    let cache = CodeIntelCache::new();
    cache.get_index(dir.path()).unwrap();
    fs::remove_file(&f).unwrap();
    let idx = cache.get_index(dir.path()).unwrap();
    assert!(!idx.definitions.iter().any(|d| d.name == "a"));
    assert!(idx.definitions.iter().any(|d| d.name == "k"));
}

#[test]
fn invalidate_files_forces_reparse_even_with_same_stat() {
    // invalidate_files must drop the entry outright: same-second same-size
    // edits are exactly the case mtime+size cannot see.
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("lib.rs");
    fs::write(&f, "fn aa() {}\n").unwrap();
    let cache = CodeIntelCache::new();
    cache.get_index(dir.path()).unwrap();
    fs::write(&f, "fn bb() {}\n").unwrap(); // same length; mtime may collide
    cache.invalidate_files(dir.path(), &["lib.rs".to_string()]);
    let idx = cache.get_index(dir.path()).unwrap();
    assert!(idx.definitions.iter().any(|d| d.name == "bb"));
}

#[test]
fn lru_evicts_past_eight_roots() {
    let cache = CodeIntelCache::new();
    let dirs: Vec<_> = (0..9).map(|_| tempfile::tempdir().unwrap()).collect();
    for d in &dirs { fs::write(d.path().join("a.rs"), "fn a() {}\n").unwrap(); cache.get_index(d.path()).unwrap(); }
    let first = cache.get_index(dirs[0].path()).unwrap(); // evicted → rebuilt, still correct
    assert!(first.definitions.iter().any(|d| d.name == "a"));
}

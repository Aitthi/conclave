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

#[cfg(unix)]
#[test]
fn unreadable_but_present_file_surfaces_in_warnings_not_dropped() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let ok = dir.path().join("ok.rs");
    let bad = dir.path().join("bad.rs");
    fs::write(&ok, "fn ok_fn() {}\n").unwrap();
    fs::write(&bad, "fn bad_fn() {}\n").unwrap();

    let cache = CodeIntelCache::new();
    let idx = cache.get_index(dir.path()).unwrap();
    assert!(idx.definitions.iter().any(|d| d.name == "ok_fn"));
    assert!(idx.definitions.iter().any(|d| d.name == "bad_fn"));

    let orig_perms = fs::metadata(&bad).unwrap().permissions();
    fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).unwrap();

    // If we can still open the file for reading despite the 0o000 bits
    // (e.g. this test is running as root, or as a user with an ACL/
    // capability override), permission enforcement isn't actually in
    // effect here and the rest of this test's assumptions don't hold.
    // Restore permissions and skip rather than assert on a false premise.
    if fs::File::open(&bad).is_ok() {
        fs::set_permissions(&bad, orig_perms).unwrap();
        eprintln!(
            "unreadable_but_present_file_surfaces_in_warnings_not_dropped: \
             0o000 file still readable (running as root?) — skipping"
        );
        return;
    }

    // chmod alone doesn't change mtime/size, so the cache's stat fast path
    // wouldn't even reattempt the read — force the stat/hash path the same
    // way an editor's save-in-place would (a real cache-entry invalidation),
    // without needing to fight filesystem mtime resolution granularity.
    cache.invalidate_files(dir.path(), &["bad.rs".to_string()]);

    let result = cache.get_index(dir.path());

    // Restore permissions before touching `result` — tempdir cleanup can
    // fail to remove a 0o000 file on some platforms, and we don't want a
    // panicking assertion below to skip this.
    fs::set_permissions(&bad, orig_perms).unwrap();

    let idx = result.expect("get_index must still return Ok for a present-but-unreadable file");
    assert!(idx.definitions.iter().any(|d| d.name == "ok_fn"));
    assert!(
        !idx.definitions.iter().any(|d| d.name == "bad_fn"),
        "bad.rs's definitions must not survive in the index once unreadable"
    );
    assert!(
        idx.warnings.iter().any(|w| w.contains("bad.rs")),
        "warnings must mention bad.rs, got: {:?}",
        idx.warnings
    );
}

#[test]
fn lru_evicts_past_eight_roots() {
    let cache = CodeIntelCache::new();
    let dirs: Vec<_> = (0..9).map(|_| tempfile::tempdir().unwrap()).collect();
    for d in &dirs { fs::write(d.path().join("a.rs"), "fn a() {}\n").unwrap(); cache.get_index(d.path()).unwrap(); }
    let first = cache.get_index(dirs[0].path()).unwrap(); // evicted → rebuilt, still correct
    assert!(first.definitions.iter().any(|d| d.name == "a"));
}

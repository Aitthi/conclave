use codeintel::cache::CodeIntelCache;
use codeintel::index::build_index;
use std::fs;

#[test]
fn definitions_carry_end_line_and_signature() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n",
    )
    .unwrap();
    let idx = build_index(dir.path()).unwrap();
    let def = idx.definitions.iter().find(|d| d.name == "greet").unwrap();
    assert_eq!(def.line, 1);
    assert_eq!(def.end_line, 3);
    assert_eq!(def.signature.as_deref(), Some("pub fn greet(name: &str) -> String {"));
}

#[test]
fn file_meta_carries_lines_and_language() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let idx = build_index(dir.path()).unwrap();
    let meta = idx.file_meta.get("lib.rs").unwrap();
    assert_eq!(meta.lines, 2);
    assert_eq!(meta.language, "rust");
}

#[test]
fn unparseable_file_lands_in_warnings() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("ok.rs"), "fn a() {}\n").unwrap();
    fs::write(dir.path().join("bad.rs"), [0xFF, 0xFE, 0x00, 0xD8]).unwrap(); // invalid UTF-8
    let idx = build_index(dir.path()).unwrap();
    assert!(idx.definitions.iter().any(|d| d.name == "a"));
    assert!(idx.warnings.iter().any(|w| w.contains("bad.rs")), "warnings: {:?}", idx.warnings);
}

#[test]
fn signature_is_truncated_to_120_chars() {
    let dir = tempfile::tempdir().unwrap();
    let long = format!("fn f(a: {}) {{}}\n", "u8, ".repeat(60));
    fs::write(dir.path().join("lib.rs"), long).unwrap();
    let idx = build_index(dir.path()).unwrap();
    let def = idx.definitions.iter().find(|d| d.name == "f").unwrap();
    assert_eq!(def.signature.as_ref().unwrap().chars().count(), 120);
}

/// `assemble_index` must sort partials by path *components* (matching
/// `walk_sources`'s `PathBuf::cmp`), not by raw `String` byte comparison.
/// Byte-wise, `"a-b/x.rs"` sorts before `"a/y.rs"` (`-` is 0x2D, `/` is
/// 0x2F), but component-wise `a` sorts before `a-b`, so `a/y.rs` comes
/// first. Regression for a divergence between `assemble_index`'s sort key
/// and `walk_sources`'s.
#[test]
fn assemble_index_orders_definitions_by_path_components_not_bytes() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a-b")).unwrap();
    fs::create_dir_all(dir.path().join("a")).unwrap();
    fs::write(dir.path().join("a-b/x.rs"), "fn x_marker() {}\n").unwrap();
    fs::write(dir.path().join("a/y.rs"), "fn y_marker() {}\n").unwrap();

    let idx = build_index(dir.path()).unwrap();
    let names: Vec<&str> = idx.definitions.iter().map(|d| d.name.as_str()).collect();
    let x_pos = names.iter().position(|n| *n == "x_marker").expect("x_marker present");
    let y_pos = names.iter().position(|n| *n == "y_marker").expect("y_marker present");
    assert!(
        y_pos < x_pos,
        "expected y_marker (a/y.rs) before x_marker (a-b/x.rs), got order {:?}",
        names
    );
}

/// Same ordering guarantee, but through `CodeIntelCache::get_index`'s
/// reassembly path (`assemble_index` invoked after an edit forces
/// `any_change`), not just the one-shot `build_index` path.
#[test]
fn cache_assembled_index_orders_definitions_by_path_components_not_bytes() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("a-b")).unwrap();
    fs::create_dir_all(dir.path().join("a")).unwrap();
    fs::write(dir.path().join("a-b/x.rs"), "fn x_marker() {}\n").unwrap();

    let cache = CodeIntelCache::new();
    // First build: only a-b/x.rs exists.
    cache.get_index(dir.path()).unwrap();

    // Add a/y.rs and re-fetch: this is a new file, so `any_change` is set
    // and `get_index` re-invokes `assemble_index` over the merged partials.
    fs::write(dir.path().join("a/y.rs"), "fn y_marker() {}\n").unwrap();
    let idx = cache.get_index(dir.path()).unwrap();

    let names: Vec<&str> = idx.definitions.iter().map(|d| d.name.as_str()).collect();
    let x_pos = names.iter().position(|n| *n == "x_marker").expect("x_marker present");
    let y_pos = names.iter().position(|n| *n == "y_marker").expect("y_marker present");
    assert!(
        y_pos < x_pos,
        "expected y_marker (a/y.rs) before x_marker (a-b/x.rs), got order {:?}",
        names
    );
}

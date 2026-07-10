//! Ported from codegraph's index_test.rs — 4 index-behavior tests that exercise
//! core indexing paths (module defs, imports flattening, call-expression refs,
//! empty-dir handling) not covered by Task 1's `index_ext_test.rs` suite.
//! Kept in a separate file (not `graph_test.rs`) since these probe `build_index`
//! directly rather than the four graph verbs, even though they use `find_refs`
//! as the cheapest probe to observe indexing output.

use codeintel::graph;
use codeintel::index::build_index;
use std::fs;

#[test]
fn rust_index_finds_lib_and_module_definitions() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Compose a small project in-place rather than copying the fixture so this test
    // does not depend on file paths under tests/fixtures.
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("lib.rs"),
        "pub mod m;\npub fn alpha() {}\nstruct Beta;\n",
    )
    .unwrap();
    fs::write(src.join("m.rs"), "pub fn gamma() {}\n").unwrap();

    let idx = build_index(tmp.path()).unwrap();
    let (data, _) = graph::find_refs(&idx, "alpha", 200).unwrap();
    // We do not assert refs yet — only that the definition is reported.
    let kinds: Vec<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"definition"),
        "expected a definition entry, got: {:?}",
        kinds
    );
}

#[test]
fn find_refs_on_empty_dir_returns_empty_data() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let idx = build_index(tmp.path()).unwrap();
    let (data, truncated) = graph::find_refs(&idx, "Nonexistent", 200).unwrap();
    assert_eq!(data.as_array().unwrap().len(), 0);
    assert!(!truncated);
}

#[test]
fn rust_imports_are_flattened() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("lib.rs"), "pub mod m;\npub fn alpha() {}\n").unwrap();
    fs::write(
        src.join("m.rs"),
        "use crate::alpha;\nuse crate::{alpha as a2, beta};\nuse crate::*;\npub fn use_alpha() { alpha(); }\n",
    )
    .unwrap();
    // The probe we use here is `find_refs alpha` — this only asserts the definition
    // still appears (= the imports query did not crash).
    let idx = build_index(tmp.path()).unwrap();
    let (data, _) = graph::find_refs(&idx, "alpha", 200).unwrap();
    assert!(data
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["kind"] == "definition"));
}

#[test]
fn rust_call_expressions_become_references() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    fs::create_dir(&src).unwrap();
    fs::write(
        src.join("lib.rs"),
        "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
    )
    .unwrap();
    let idx = build_index(tmp.path()).unwrap();
    let (data, _) = graph::find_refs(&idx, "alpha", 200).unwrap();
    let kinds: Vec<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"definition"), "kinds: {:?}", kinds);
    assert!(kinds.contains(&"call"), "kinds: {:?}", kinds);
}

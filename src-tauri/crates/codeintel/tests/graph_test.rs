//! Ported from codegraph's find_refs_test.rs / callers_test.rs / callees_test.rs /
//! impact_test.rs — 13 behavioral tests, rewritten as direct library calls against
//! `graph::{find_refs,callers,callees,impact}` instead of spawning the `codegraph`
//! binary. The two CLI-only tests (`cli_help_lists_all_four_subcommands` + help text)
//! are skipped: there is no binary in this crate.

use codeintel::graph;
use codeintel::index::build_index;
use std::path::{Path, PathBuf};

fn rust_fixture() -> &'static Path {
    Path::new("tests/fixtures/multi_lang/rust_app")
}
fn ts_fixture() -> PathBuf {
    PathBuf::from("tests/fixtures/multi_lang/ts_app")
}
fn js_fixture() -> PathBuf {
    PathBuf::from("tests/fixtures/multi_lang/js_app")
}
fn py_fixture() -> PathBuf {
    PathBuf::from("tests/fixtures/multi_lang/py_app")
}

// --- find_refs ---

#[test]
fn cross_file_call_to_imported_fn_is_high_confidence() {
    let idx = build_index(rust_fixture()).unwrap();
    let (data, _) = graph::find_refs(&idx, "authenticate", 200).unwrap();
    let hits = data.as_array().unwrap();
    let cross_file_call = hits
        .iter()
        .find(|h| {
            h["file"].as_str().unwrap().ends_with("handlers.rs")
                && h["kind"].as_str().unwrap() == "call"
        })
        .expect("handlers.rs should contain a call to authenticate");
    assert_eq!(cross_file_call["confidence"].as_str().unwrap(), "high");
    assert_eq!(
        cross_file_call["reason"].as_str().unwrap(),
        "import-resolved"
    );
}

#[test]
fn same_file_call_is_high_confidence() {
    let idx = build_index(rust_fixture()).unwrap();
    let (data, _) = graph::find_refs(&idx, "authenticate", 200).unwrap();
    let hits = data.as_array().unwrap();
    let same_file_call = hits
        .iter()
        .find(|h| {
            h["file"].as_str().unwrap().ends_with("auth.rs")
                && h["kind"].as_str().unwrap() == "call"
        })
        .expect("auth.rs should call authenticate from revoke");
    assert_eq!(same_file_call["confidence"].as_str().unwrap(), "high");
    assert_eq!(
        same_file_call["reason"].as_str().unwrap(),
        "same-file-scope"
    );
}

#[test]
fn unused_helper_has_definition_but_no_calls() {
    let idx = build_index(rust_fixture()).unwrap();
    let (data, _) = graph::find_refs(&idx, "unused_helper", 200).unwrap();
    let hits = data.as_array().unwrap();
    assert!(hits.iter().any(|h| h["kind"] == "definition"));
    assert!(
        !hits.iter().any(|h| h["kind"] == "call"),
        "unused_helper should not have any call sites, got: {hits:?}"
    );
}

#[test]
fn ts_cross_file_import_is_high_confidence() {
    let idx = build_index(&ts_fixture()).unwrap();
    let (data, _) = graph::find_refs(&idx, "authenticate", 200).unwrap();
    let cross = data
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["file"].as_str().unwrap().ends_with("handlers.ts") && h["kind"] == "call")
        .expect("handlers.ts call to authenticate");
    assert_eq!(cross["confidence"], "high");
    assert_eq!(cross["reason"], "import-resolved");
}

#[test]
fn js_cross_file_import_resolves() {
    let idx = build_index(&js_fixture()).unwrap();
    let (data, _) = graph::find_refs(&idx, "add", 200).unwrap();
    let call = data
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["file"].as_str().unwrap().ends_with("index.js") && h["kind"] == "call")
        .expect("index.js call to add");
    assert_eq!(call["confidence"], "high");
}

#[test]
fn python_cross_file_import_resolves() {
    let idx = build_index(&py_fixture()).unwrap();
    let (data, _) = graph::find_refs(&idx, "authenticate", 200).unwrap();
    let cross = data
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["file"].as_str().unwrap().ends_with("handlers.py") && h["kind"] == "call")
        .expect("handlers.py call to authenticate");
    assert_eq!(cross["confidence"], "high");
    assert_eq!(cross["reason"], "import-resolved");
}

// --- callers ---

#[test]
fn callers_of_authenticate_includes_revoke_and_login() {
    let idx = build_index(rust_fixture()).unwrap();
    let data = graph::callers(&idx, "authenticate", 8).unwrap();
    let callers: Vec<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(callers.contains(&"revoke"), "callers: {callers:?}");
    assert!(callers.contains(&"login"), "callers: {callers:?}");
    // `whoami` does not call authenticate.
    assert!(!callers.contains(&"whoami"), "callers: {callers:?}");
}

#[test]
fn callers_of_unused_helper_is_empty() {
    let idx = build_index(rust_fixture()).unwrap();
    let data = graph::callers(&idx, "unused_helper", 8).unwrap();
    assert_eq!(data.as_array().unwrap().len(), 0);
}

#[test]
fn callers_depth_2_walks_one_more_hop() {
    let idx = build_index(rust_fixture()).unwrap();
    let data = graph::callers(&idx, "authenticate", 2).unwrap();
    let entries = data.as_array().unwrap();
    // login calls authenticate (distance=1); nothing calls login in this fixture,
    // so depth=2 should match depth=1 in count — but the field must be present.
    assert!(entries.iter().all(|e| e["distance"].is_number()));
    assert!(entries
        .iter()
        .any(|e| e["name"] == "revoke" && e["distance"] == 1));
}

// --- callees ---

#[test]
fn callees_of_login_includes_new_user_and_authenticate() {
    let idx = build_index(rust_fixture()).unwrap();
    let data = graph::callees(&idx, "login", 8).unwrap();
    let names: Vec<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"new_user"), "names: {names:?}");
    assert!(names.contains(&"authenticate"), "names: {names:?}");
}

#[test]
fn callees_of_missing_fn_returns_empty() {
    let idx = build_index(rust_fixture()).unwrap();
    // We treat "no such fn" as success-with-empty rather than an error, because
    // pipelines that ask "what does X call?" want a clean empty list.
    let data = graph::callees(&idx, "definitely_not_a_function", 8).unwrap();
    assert_eq!(data.as_array().unwrap().len(), 0);
}

// --- impact ---

#[test]
fn impact_of_authenticate_includes_login_and_revoke() {
    let idx = build_index(rust_fixture()).unwrap();
    let data = graph::impact(&idx, "authenticate").unwrap();
    let names: Vec<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"authenticate"));
    assert!(names.contains(&"login"));
    assert!(names.contains(&"revoke"));
}

#[test]
fn impact_of_user_struct_includes_type_position_uses() {
    let idx = build_index(rust_fixture()).unwrap();
    let data = graph::impact(&idx, "User").unwrap();
    // Every fn that takes `&User` should appear (authenticate, revoke, whoami).
    let names: Vec<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    for fname in ["authenticate", "revoke", "whoami"] {
        assert!(
            names.contains(&fname),
            "impact of User missing {fname}: {names:?}"
        );
    }
}

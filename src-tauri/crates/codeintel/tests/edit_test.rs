//! Integration tests for `codeintel::edit::{rename, rewrite}` — ported from
//! astedit's `tests/rename_test.rs` and `tests/rewrite_test.rs` (see
//! codeintel Task 5 brief). The original tests spawned the `astedit` binary
//! and asserted on `(exit_code, data)`; here we call the library functions
//! directly and assert on `(Value, Vec<String>)`. Exit-code translation:
//!
//! - old exit 2 + needs_anchor  ⇒ assert data\["needs_anchor"\] == true
//! - old pattern-compile exit 2 ⇒ assert data\["errors"\]\[0\]\["error_kind"\] == "pattern-compile"
//!
//! Every other former exit-code-0 case just asserts on the payload directly
//! (there is no longer a code to check — `rename`/`rewrite` return `Ok`
//! whenever the operation completed, per the Task 5 brief).

use std::fs;
use std::path::{Path, PathBuf};

use codeintel::edit;
use codeintel::index::build_index;
use tempfile::TempDir;

/// Copy `crates/codeintel/tests/fixtures/<name>/` into a fresh TempDir and
/// return the TempDir (so the caller controls its lifetime — drop removes
/// the copy). Ported from astedit's `tests/common/mod.rs::copy_fixture`.
fn copy_fixture(name: &str) -> TempDir {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    assert!(
        src.is_dir(),
        "fixture {:?} not found — did you forget to add it under tests/fixtures/?",
        src,
    );
    let dst = TempDir::new().expect("create tempdir");
    copy_recursive(&src, dst.path());
    dst
}

fn copy_recursive(from: &Path, to: &Path) {
    if !to.exists() {
        fs::create_dir_all(to).expect("mkdir -p tempdir target");
    }
    for entry in fs::read_dir(from).expect("read_dir fixture") {
        let entry = entry.expect("dir entry");
        let kind = entry.file_type().expect("file_type");
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if kind.is_dir() {
            copy_recursive(&src, &dst);
        } else if kind.is_file() {
            fs::copy(&src, &dst).expect("copy fixture file");
        }
        // Symlinks in fixtures are not supported (yagni).
    }
}

// ---------------------------------------------------------------------
// rename
// ---------------------------------------------------------------------

#[test]
fn rename_same_file_high_confidence_dry_run_default() {
    let tmp = copy_fixture("same_file");
    let idx = build_index(tmp.path()).unwrap();

    let (data, written) =
        edit::rename(tmp.path(), &idx, "User", "Account", false, None, None).unwrap();

    assert_eq!(data["subcommand"], "rename");
    assert_eq!(data["dry_run"], true);
    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "no errors expected: {:?}",
        data["errors"]
    );
    assert!(written.is_empty(), "dry-run must not report written files");

    let applied = data["applied"].as_array().expect("applied array");
    assert_eq!(
        applied.len(),
        1,
        "single fixture file expected; got: {applied:?}"
    );
    let file_entry = &applied[0];
    assert!(file_entry["file"].as_str().unwrap().ends_with("main.rs"));

    let edits = file_entry["edits"].as_array().unwrap();
    // The fixture has the struct DEFINITION (1) + 3 use sites = 4 identifier
    // sites total. The resolver returns references (not definitions); whether
    // the definition's identifier is itself a Reference depends on the index.
    // The test asserts at least 3 (the use sites) and at most 4.
    assert!(
        edits.len() >= 3 && edits.len() <= 4,
        "expected 3-4 edits, got {}",
        edits.len()
    );

    for e in edits {
        assert_eq!(e["old"], "User");
        assert_eq!(e["new"], "Account");
        assert_eq!(e["confidence"], "high");
        assert_eq!(e["reason"], "same-file-scope");
        assert!(e["start_byte"].as_u64().unwrap() < e["end_byte"].as_u64().unwrap());
        assert_eq!(
            e["end_byte"].as_u64().unwrap() - e["start_byte"].as_u64().unwrap(),
            "User".len() as u64,
        );
    }

    // Dry-run must NOT mutate the fixture copy.
    let fixture_file = tmp.path().join("main.rs");
    let after = fs::read_to_string(&fixture_file).unwrap();
    assert!(
        after.contains("struct User"),
        "dry-run modified the file: {after}"
    );
    assert!(
        !after.contains("Account"),
        "dry-run wrote Account into the file"
    );
}

#[test]
fn rename_cross_file_import_resolved() {
    let tmp = copy_fixture("cross_file_import");
    let idx = build_index(tmp.path()).unwrap();

    let (data, _written) =
        edit::rename(tmp.path(), &idx, "User", "Account", false, None, None).unwrap();

    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "errors: {:?}",
        data["errors"]
    );

    let applied = data["applied"].as_array().expect("applied array");
    // Expect at least src/lib.rs to be touched (the import-resolved use).
    // src/inner.rs contains the definition itself; whether its self-references
    // count depends on the indexer — accept 1 or 2 files.
    assert!(
        !applied.is_empty() && applied.len() <= 2,
        "got {} files: {applied:?}",
        applied.len()
    );

    let files: Vec<&str> = applied
        .iter()
        .map(|f| f["file"].as_str().unwrap())
        .collect();
    assert!(
        files.iter().any(|f| f.ends_with("lib.rs")),
        "lib.rs not in applied: {files:?}",
    );

    let lib = applied
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with("lib.rs"))
        .unwrap();
    let lib_edits = lib["edits"].as_array().unwrap();
    assert!(
        !lib_edits.is_empty(),
        "expected at least one edit in lib.rs"
    );
    for e in lib_edits {
        assert_eq!(e["old"], "User");
        assert_eq!(e["new"], "Account");
        // import-resolved across files is "high" via ImportResolved reason.
        assert_eq!(e["reason"], "import-resolved", "got {e:?}");
        assert_eq!(e["confidence"], "high", "got {e:?}");
    }
}

#[test]
fn rename_glob_import_medium_confidence_applied() {
    let tmp = copy_fixture("glob_import");
    let idx = build_index(tmp.path()).unwrap();

    let (data, _written) =
        edit::rename(tmp.path(), &idx, "User", "Account", false, None, None).unwrap();

    let applied = data["applied"].as_array().expect("applied array");

    let lib = applied
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with("lib.rs"))
        .expect("lib.rs should be in applied");
    let lib_edits = lib["edits"].as_array().unwrap();
    assert!(
        !lib_edits.is_empty(),
        "expected at least one edit in lib.rs"
    );
    for e in lib_edits {
        // Glob-only import → resolver assigns Medium.
        assert_eq!(
            e["confidence"], "medium",
            "expected medium for glob import: {e:?}"
        );
    }
}

#[test]
fn rename_name_only_goes_to_skipped_low_confidence() {
    let tmp = copy_fixture("name_only");
    let idx = build_index(tmp.path()).unwrap();

    let (data, _written) =
        edit::rename(tmp.path(), &idx, "User", "Account", false, None, None).unwrap();

    let skipped = data["skipped"].as_array().expect("skipped array");
    let lows: Vec<&serde_json::Value> = skipped
        .iter()
        .filter(|s| s["skip_reason"] == "low-confidence")
        .collect();
    assert!(
        !lows.is_empty(),
        "expected at least one low-confidence skip; skipped: {skipped:?}"
    );

    for s in &lows {
        assert_eq!(s["confidence"], "low");
        assert_eq!(s["reason"], "name-only");
        assert_eq!(s["name"], "User");
        assert!(s["file"].as_str().unwrap().ends_with("unrelated.rs"));
    }

    // unrelated.rs must NOT appear in applied.
    let applied = data["applied"].as_array().unwrap();
    let bad = applied
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with("unrelated.rs"));
    assert!(
        bad.is_none(),
        "unrelated.rs should not be in applied: {bad:?}"
    );
}

#[test]
fn rename_alias_reexport_skipped_with_via_alias() {
    let tmp = copy_fixture("alias_reexport");
    let idx = build_index(tmp.path()).unwrap();

    let (data, _written) =
        edit::rename(tmp.path(), &idx, "User", "Account", false, None, None).unwrap();

    let skipped = data["skipped"].as_array().expect("skipped array");
    let aliases: Vec<&serde_json::Value> = skipped
        .iter()
        .filter(|s| s["skip_reason"] == "re-export-alias")
        .collect();
    assert_eq!(aliases.len(), 1, "expected one alias skip; got {skipped:?}");

    let alias = aliases[0];
    assert!(alias["file"].as_str().unwrap().ends_with("lib.rs"));
    assert_eq!(alias["name"], "User");
    assert_eq!(
        alias["via_alias"], "Bar",
        "via_alias should be the original symbol"
    );
    match alias.get("via_module") {
        None => {}
        Some(v) if v.is_null() => {}
        Some(other) => panic!("via_module must not appear on re-export-alias entries: {other:?}"),
    }
}

#[test]
fn rename_wildcard_reexport_skipped_with_via_module() {
    let tmp = copy_fixture("wildcard_reexport");
    let idx = build_index(tmp.path()).unwrap();

    let (data, _written) =
        edit::rename(tmp.path(), &idx, "User", "Account", false, None, None).unwrap();

    let skipped = data["skipped"].as_array().expect("skipped array");
    let wilds: Vec<&serde_json::Value> = skipped
        .iter()
        .filter(|s| s["skip_reason"] == "wildcard-reexport")
        .collect();
    assert_eq!(
        wilds.len(),
        1,
        "expected one wildcard skip; got {skipped:?}"
    );

    let w = wilds[0];
    assert!(w["file"].as_str().unwrap().ends_with("lib.rs"));
    assert!(
        w["via_module"].as_str().unwrap().contains("inner"),
        "via_module should reference inner module; got {:?}",
        w["via_module"]
    );
    match w.get("via_alias") {
        None => {}
        Some(v) if v.is_null() => {}
        Some(other) => panic!("via_alias must not appear on wildcard-reexport entries: {other:?}"),
    }
}

#[test]
fn rename_multi_def_without_anchor_needs_anchor_payload() {
    let tmp = copy_fixture("multi_def");
    let idx = build_index(tmp.path()).unwrap();

    let (data, written) =
        edit::rename(tmp.path(), &idx, "User", "Account", false, None, None).unwrap();

    // Former exit code 2 (needs_anchor) is now fully represented in the
    // payload; the call still returns Ok.
    assert_eq!(data["subcommand"], "rename");
    assert_eq!(data["needs_anchor"], true);
    assert!(written.is_empty());

    let candidates = data["candidates"].as_array().expect("candidates array");
    assert_eq!(
        candidates.len(),
        2,
        "expected exactly two candidates: {candidates:?}"
    );

    let kinds: Vec<&str> = candidates
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"struct"));
    assert!(kinds.contains(&"fn"));

    for c in candidates {
        assert!(c["line"].as_u64().unwrap() >= 1);
        let f = c["file"].as_str().unwrap();
        assert!(f.ends_with("a.rs") || f.ends_with("b.rs"));
    }

    // No applied/skipped/errors on the needs_anchor path.
    match data.get("applied") {
        None => {}
        Some(v) if v.is_null() => {}
        Some(other) => panic!("applied should be absent: {other:?}"),
    }
    match data.get("skipped") {
        None => {}
        Some(v) if v.is_null() => {}
        Some(other) => panic!("skipped should be absent: {other:?}"),
    }
    match data.get("errors") {
        None => {}
        Some(v) if v.is_null() => {}
        Some(other) => panic!("errors should be absent: {other:?}"),
    }
}

#[test]
fn rename_with_anchor_picks_matching_definition() {
    let tmp = copy_fixture("multi_def");
    let idx = build_index(tmp.path()).unwrap();

    let (data, _written) = edit::rename(
        tmp.path(),
        &idx,
        "User",
        "Account",
        false,
        None,
        Some("src/a.rs:1"),
    )
    .unwrap();

    match data.get("needs_anchor") {
        None => {}
        Some(v) if v.is_null() => {}
        Some(other) => panic!("needs_anchor should be absent: {other:?}"),
    }

    // The `fn User` in b.rs must not be touched — its references should not
    // appear in applied.
    let applied = data["applied"].as_array().expect("applied array");
    for f in applied {
        let file = f["file"].as_str().unwrap();
        assert!(
            file.ends_with("a.rs") || file.ends_with("lib.rs"),
            "anchor picked struct in a.rs; b.rs should not appear: {file}"
        );
    }
    // Specifically: b.rs must NOT be in applied
    let bad = applied
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with("b.rs"));
    assert!(
        bad.is_none(),
        "b.rs should not be in applied with anchor a.rs:1: {bad:?}"
    );
}

#[test]
fn rename_apply_writes_changes_to_disk() {
    let tmp = copy_fixture("apply_write");
    let target = tmp.path().join("main.rs");
    let before = fs::read_to_string(&target).unwrap();
    assert!(before.contains("struct User"));

    let idx = build_index(tmp.path()).unwrap();
    let (data, written) =
        edit::rename(tmp.path(), &idx, "User", "Account", true, None, None).unwrap();

    assert_eq!(data["dry_run"], false);
    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "errors: {:?}",
        data["errors"]
    );
    assert!(
        written.iter().any(|f| f.ends_with("main.rs")),
        "expected main.rs among written files: {written:?}"
    );

    let after = fs::read_to_string(&target).unwrap();
    assert!(!after.contains("struct User"), "User should be renamed");
    assert!(
        after.contains("struct Account"),
        "Account should appear: {after}"
    );
    assert!(
        after.contains("Account { id: 42 }"),
        "body literal not renamed: {after}"
    );
    assert!(
        after.contains("-> Account"),
        "return type not renamed: {after}"
    );

    // `bytes_changed` should be (new - old) * #edits, where new=7, old=4, delta=+3.
    let applied = data["applied"].as_array().unwrap();
    let entry = applied
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with("main.rs"))
        .unwrap();
    let bytes_changed = entry["bytes_changed"].as_i64().unwrap();
    let edit_count = entry["edits"].as_array().unwrap().len() as i64;
    assert_eq!(
        bytes_changed,
        3 * edit_count,
        "bytes_changed should be 3 per edit"
    );
}

#[test]
fn rename_fn_apply_edits_definition_site() {
    // Regression for ruling 77b4ae3d: fn definitions are not captured as
    // references (unlike struct type_identifiers), so --apply used to rename
    // the call sites but leave `fn old_name` behind — broken code.
    let tmp = copy_fixture("fn_rename");
    let target = tmp.path().join("main.rs");
    let before = fs::read_to_string(&target).unwrap();
    assert!(before.contains("fn old_name"));

    let idx = build_index(tmp.path()).unwrap();
    let (data, written) =
        edit::rename(tmp.path(), &idx, "old_name", "new_name", true, None, None).unwrap();

    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "errors: {:?}",
        data["errors"]
    );
    assert!(
        written.iter().any(|f| f.ends_with("main.rs")),
        "expected main.rs among written files: {written:?}"
    );

    let after = fs::read_to_string(&target).unwrap();
    assert!(
        after.contains("fn new_name"),
        "definition site not renamed: {after}"
    );
    assert!(
        after.contains("new_name()"),
        "call site not renamed: {after}"
    );
    assert!(
        !after.contains("old_name"),
        "old_name must not survive anywhere: {after}"
    );
}

#[test]
fn rename_fn_dry_run_reports_def_site_edit() {
    let tmp = copy_fixture("fn_rename");
    let idx = build_index(tmp.path()).unwrap();

    let (data, written) =
        edit::rename(tmp.path(), &idx, "old_name", "new_name", false, None, None).unwrap();
    assert!(written.is_empty(), "dry-run must not write");

    let applied = data["applied"].as_array().expect("applied array");
    let entry = applied
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with("main.rs"))
        .expect("main.rs in applied");
    let edits = entry["edits"].as_array().unwrap();

    // `fn old_name` starts the file, so the name token spans bytes 3..11.
    let def_edit = edits
        .iter()
        .find(|e| e["reason"] == "definition")
        .unwrap_or_else(|| panic!("no definition-site edit reported: {edits:?}"));
    assert_eq!(def_edit["start_byte"], 3, "got {def_edit:?}");
    assert_eq!(def_edit["end_byte"], 11, "got {def_edit:?}");
    assert_eq!(def_edit["old"], "old_name");
    assert_eq!(def_edit["new"], "new_name");
    assert_eq!(def_edit["confidence"], "high");

    // The same-file call site is still reported alongside it.
    assert!(
        edits.iter().any(|e| e["reason"] == "same-file-scope"),
        "call-site edit missing: {edits:?}"
    );

    // Dry-run must not mutate the fixture copy.
    let after = fs::read_to_string(tmp.path().join("main.rs")).unwrap();
    assert!(after.contains("fn old_name"), "dry-run modified the file");
}

#[test]
fn drift_between_index_and_apply_emits_hash_mismatch() {
    let tmp = copy_fixture("apply_write");

    // Build an index against the fresh fixture.
    let index = build_index(tmp.path()).unwrap();
    // file_meta keys are repo-relative, forward-slash-normalized.
    let (rel, meta) = index
        .file_meta
        .iter()
        .find(|(k, _)| k.ends_with("main.rs"))
        .expect("main.rs in file_meta");
    let original_len = meta.len;

    // Mutate the file so its length differs from the snapshot.
    let target = tmp.path().join("main.rs");
    let mut content = fs::read_to_string(&target).unwrap();
    content.push_str("\n// drift bait\n");
    fs::write(&target, &content).unwrap();
    assert_ne!(content.len() as u64, original_len);

    // The drift checker now sees length mismatch + no recorded hash → error.
    let err =
        edit::apply::check_drift(&target, rel, meta, None).expect_err("expected hash-mismatch");
    assert_eq!(err.kind(), "hash-mismatch");
}

// ---------------------------------------------------------------------
// rewrite
// ---------------------------------------------------------------------

#[test]
fn rewrite_rust_two_matches_dry_run_default() {
    let tmp = copy_fixture("rewrite_rust");

    let (data, written) =
        edit::rewrite(tmp.path(), "println!($A)", "eprintln!($A)", false, None).unwrap();

    assert_eq!(data["subcommand"], "rewrite");
    assert_eq!(data["dry_run"], true);
    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "no errors expected: {:?}",
        data["errors"]
    );
    assert!(written.is_empty(), "dry-run must not report written files");

    let applied = data["applied"].as_array().expect("applied array");
    assert_eq!(
        applied.len(),
        1,
        "single fixture file expected; got: {applied:?}"
    );

    let file_entry = &applied[0];
    assert!(file_entry["file"].as_str().unwrap().ends_with("main.rs"));

    let edits = file_entry["edits"].as_array().unwrap();
    assert_eq!(
        edits.len(),
        2,
        "expected 2 println matches; got {}",
        edits.len()
    );

    for e in edits {
        assert!(e["old"].as_str().unwrap().starts_with("println!"));
        assert!(e["new"].as_str().unwrap().starts_with("eprintln!"));
        assert!(
            e.get("confidence").is_none(),
            "rewrite edit must not have confidence: {e:?}"
        );
        assert!(
            e.get("reason").is_none(),
            "rewrite edit must not have reason: {e:?}"
        );
        assert!(e["start_byte"].as_u64().unwrap() < e["end_byte"].as_u64().unwrap());
    }

    let fixture_file = tmp.path().join("main.rs");
    let after = fs::read_to_string(&fixture_file).unwrap();
    assert!(
        after.contains("println!(\"hi\")"),
        "dry-run modified the fixture: {after}"
    );
    assert!(
        !after.contains("eprintln"),
        "dry-run wrote eprintln! into the fixture"
    );
}

#[test]
fn rewrite_typescript_matches() {
    let tmp = copy_fixture("rewrite_typescript");

    let (data, _written) = edit::rewrite(
        tmp.path(),
        "console.log($A)",
        "console.error($A)",
        false,
        None,
    )
    .unwrap();

    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "errors: {:?}",
        data["errors"]
    );

    let applied = data["applied"].as_array().expect("applied array");
    assert_eq!(applied.len(), 1);
    assert!(applied[0]["file"].as_str().unwrap().ends_with("main.ts"));

    let edits = applied[0]["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 2, "two console.log calls expected");
    for e in edits {
        assert!(e["old"].as_str().unwrap().starts_with("console.log"));
        assert!(e["new"].as_str().unwrap().starts_with("console.error"));
    }
}

#[test]
fn rewrite_tsx_matches_jsx_file() {
    let tmp = copy_fixture("rewrite_tsx");

    let (data, _written) = edit::rewrite(
        tmp.path(),
        "console.log($A)",
        "console.error($A)",
        false,
        None,
    )
    .unwrap();

    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "errors: {:?}",
        data["errors"]
    );
    let applied = data["applied"].as_array().expect("applied array");
    assert_eq!(applied.len(), 1, "exactly one .tsx file expected");
    assert!(applied[0]["file"].as_str().unwrap().ends_with("main.tsx"));
    assert_eq!(applied[0]["edits"].as_array().unwrap().len(), 1);
}

#[test]
fn rewrite_javascript_matches() {
    let tmp = copy_fixture("rewrite_javascript");

    let (data, _written) = edit::rewrite(
        tmp.path(),
        "console.log($A)",
        "console.error($A)",
        false,
        None,
    )
    .unwrap();

    let applied = data["applied"].as_array().expect("applied array");
    assert_eq!(applied.len(), 1);
    assert!(applied[0]["file"].as_str().unwrap().ends_with("main.js"));
    assert_eq!(applied[0]["edits"].as_array().unwrap().len(), 1);
}

#[test]
fn rewrite_python_matches() {
    let tmp = copy_fixture("rewrite_python");

    let (data, _written) =
        edit::rewrite(tmp.path(), "print($A)", "logging.info($A)", false, None).unwrap();

    let applied = data["applied"].as_array().expect("applied array");
    assert_eq!(applied.len(), 1);
    assert!(applied[0]["file"].as_str().unwrap().ends_with("main.py"));
    assert_eq!(applied[0]["edits"].as_array().unwrap().len(), 2);
    for e in applied[0]["edits"].as_array().unwrap() {
        assert!(e["new"].as_str().unwrap().starts_with("logging.info"));
    }
}

#[test]
fn rewrite_single_metavar_substituted_in_replacement() {
    let tmp = copy_fixture("rewrite_metavar");

    let (data, _written) = edit::rewrite(
        tmp.path(),
        "String::from($S)",
        "String::from($S.to_owned())",
        false,
        None,
    )
    .unwrap();

    let applied = data["applied"].as_array().unwrap();
    assert_eq!(applied.len(), 1);
    let edits = applied[0]["edits"].as_array().unwrap();
    assert_eq!(edits.len(), 2);

    let news: Vec<&str> = edits.iter().map(|e| e["new"].as_str().unwrap()).collect();
    assert!(
        news.iter().any(|n| n.contains("\"alice\".to_owned()")),
        "expected `alice` capture materialised in rewrite; news = {news:?}",
    );
    assert!(
        news.iter().any(|n| n.contains("\"bob\".to_owned()")),
        "expected `bob` capture materialised in rewrite; news = {news:?}",
    );
}

#[test]
fn rewrite_multimatch_metavar_preserves_argument_list() {
    let tmp = copy_fixture("rewrite_multimatch");

    let (data, _written) = edit::rewrite(
        tmp.path(),
        "log($$$ARGS)",
        "tracing::info!($$$ARGS)",
        false,
        None,
    )
    .unwrap();

    let applied = data["applied"].as_array().unwrap();
    assert_eq!(
        applied.len(),
        1,
        "single-file fixture expected; got: {applied:?}"
    );

    let edits = applied[0]["edits"].as_array().unwrap();
    assert_eq!(
        edits.len(),
        2,
        "expected 2 call-site matches; edits={edits:?}"
    );

    let news: Vec<&str> = edits.iter().map(|e| e["new"].as_str().unwrap()).collect();
    assert!(
        news.iter().any(|n| n.contains("\"a\", 1, true")),
        "expected 3-arg capture preserved; news = {news:?}"
    );
    assert!(
        news.iter().any(|n| n.contains("(\"b\")")),
        "expected single-arg capture preserved; news = {news:?}"
    );
}

#[test]
fn rewrite_no_match_reports_empty_applied() {
    let tmp = copy_fixture("rewrite_no_match");

    let (data, written) =
        edit::rewrite(tmp.path(), "println!($A)", "eprintln!($A)", false, None).unwrap();

    assert_eq!(data["subcommand"], "rewrite");
    assert_eq!(data["dry_run"], true);
    assert!(written.is_empty());
    assert!(
        data["applied"].as_array().unwrap().is_empty(),
        "applied must be empty when no matches; got: {:?}",
        data["applied"]
    );
    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "errors must be empty when no matches; got: {:?}",
        data["errors"]
    );
}

#[test]
fn rewrite_lang_filter_unset_walks_all_languages() {
    let tmp = copy_fixture("rewrite_lang_filter");

    let (data, _written) = edit::rewrite(tmp.path(), "doit()", "done()", false, None).unwrap();

    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "errors: {:?}",
        data["errors"]
    );

    let applied = data["applied"].as_array().unwrap();
    let rs_present = applied
        .iter()
        .any(|f| f["file"].as_str().unwrap().ends_with("main.rs"));
    let py_present = applied
        .iter()
        .any(|f| f["file"].as_str().unwrap().ends_with("main.py"));
    assert!(rs_present, "main.rs missing from applied: {applied:?}");
    assert!(py_present, "main.py missing from applied: {applied:?}");
}

#[test]
fn rewrite_lang_rust_skips_non_rust_files_entirely() {
    let tmp = copy_fixture("rewrite_lang_filter");

    let (data, _written) =
        edit::rewrite(tmp.path(), "doit()", "done()", false, Some("rust")).unwrap();

    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "errors should be empty — python file was filtered out, not processed: {:?}",
        data["errors"]
    );

    let applied = data["applied"].as_array().unwrap();
    let files: Vec<&str> = applied
        .iter()
        .map(|f| f["file"].as_str().unwrap())
        .collect();
    assert!(
        files.iter().any(|f| f.ends_with("main.rs")),
        "rust file should be applied: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f.ends_with("main.py")),
        "python file must not appear when --lang rust filters it out: {files:?}"
    );
}

#[test]
fn rewrite_pattern_compile_failure_reports_error_kind() {
    let tmp = copy_fixture("rewrite_rust");

    let (data, written) =
        edit::rewrite(tmp.path(), "(((", "eprintln!($A)", false, Some("rust")).unwrap();

    // Former exit code 2 (pattern-compile) is now fully represented in the
    // payload; the call still returns Ok.
    assert_eq!(data["subcommand"], "rewrite");
    assert!(written.is_empty());

    let errors = data["errors"].as_array().expect("errors array");
    let pattern_errs: Vec<&serde_json::Value> = errors
        .iter()
        .filter(|e| e["error_kind"] == "pattern-compile")
        .collect();
    assert!(
        !pattern_errs.is_empty(),
        "expected at least one pattern-compile error; errors = {errors:?}"
    );

    let err = pattern_errs[0];
    assert_eq!(err["lang"], "rust");
}

#[test]
fn rewrite_apply_writes_changes_to_disk() {
    let tmp = copy_fixture("rewrite_apply");
    let target = tmp.path().join("main.rs");
    let before = fs::read_to_string(&target).unwrap();
    assert!(before.contains("println!"));

    let (data, written) =
        edit::rewrite(tmp.path(), "println!($A)", "eprintln!($A)", true, None).unwrap();

    assert_eq!(data["dry_run"], false);
    assert!(
        data["errors"].as_array().unwrap().is_empty(),
        "no errors expected; got: {:?}",
        data["errors"]
    );
    assert!(
        written.iter().any(|f| f.ends_with("main.rs")),
        "expected main.rs among written files: {written:?}"
    );

    let after = fs::read_to_string(&target).unwrap();
    assert_ne!(
        before, after,
        "file should have been modified by apply: before={before}; after={after}"
    );
    assert!(
        after.contains("eprintln!(\"a\")"),
        "first eprintln should be present: {after}"
    );
    assert!(
        after.contains("eprintln!(\"bb\")"),
        "second eprintln should be present: {after}"
    );

    let applied = data["applied"].as_array().unwrap();
    let entry = applied
        .iter()
        .find(|f| f["file"].as_str().unwrap().ends_with("main.rs"))
        .unwrap();
    let bytes_changed = entry["bytes_changed"].as_i64().unwrap();
    assert_eq!(
        bytes_changed, 2,
        "expected +2 bytes total; got {bytes_changed}"
    );
}

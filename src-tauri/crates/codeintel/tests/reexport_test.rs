use codeintel::index::build_index;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn rust_alias_reexport_recorded() {
    let index = build_index(&fixture("reexport_rust")).unwrap();
    let sites = index
        .alias_reexports
        .get("Baz")
        .expect("Baz alias should be recorded under its local name");
    assert_eq!(sites.len(), 1);
    let site = &sites[0];
    assert!(site.file.ends_with("lib.rs"), "got {}", site.file);
    assert_eq!(site.alias, "Baz");
    assert_eq!(site.original, "Bar");
    assert!(site.module_path.contains("inner"));
}

#[test]
fn rust_wildcard_reexport_recorded() {
    let index = build_index(&fixture("reexport_rust")).unwrap();
    let sites = index
        .wildcard_reexports
        .get("widgets")
        .expect("widgets wildcard re-export should be recorded");
    assert_eq!(sites.len(), 1);
    assert!(sites[0].file.ends_with("lib.rs"));
    assert!(sites[0].module_path.contains("inner::widgets"));
}

#[test]
fn rust_non_pub_use_is_not_a_reexport() {
    // `use inner::Untouched;` (no `pub`) is a plain import, not a re-export.
    // It must NOT appear in either re-export table.
    let index = build_index(&fixture("reexport_rust")).unwrap();
    assert!(
        index
            .alias_reexports
            .values()
            .flat_map(|v| v.iter())
            .all(|s| s.alias != "Untouched"),
        "Untouched is a pub use without alias — should not be in alias_reexports",
    );
}

#[test]
fn rust_direct_reexport_stays_in_imports() {
    // `pub use inner::Untouched;` is a direct re-export (no alias, no glob).
    // Per spec ("Direct re-exports are treated like any other reference"), it
    // MUST remain in the imports table so the resolver can trace references
    // through it. The duplicate-prevention rule introduced in Task 13 must NOT
    // drop this entry.
    let index = build_index(&fixture("reexport_rust")).unwrap();
    let found = index
        .imports
        .iter()
        .any(|imp| imp.local_name == "Untouched" || imp.imported_name == "Untouched");
    assert!(
        found,
        "pub use inner::Untouched; (direct re-export, no alias) must be recorded in imports. \
         Got imports: {:?}",
        index
            .imports
            .iter()
            .map(|i| format!("{}::{}", i.module_path, i.local_name))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ts_alias_reexport_recorded() {
    let index = build_index(&fixture("reexport_ts")).unwrap();
    let sites = index
        .alias_reexports
        .get("Baz")
        .expect("Baz alias should be recorded");
    assert_eq!(sites.len(), 1);
    assert!(sites[0].file.ends_with("lib.ts"));
    assert_eq!(sites[0].alias, "Baz");
    assert_eq!(sites[0].original, "Bar");
    assert_eq!(sites[0].module_path, "./inner");
}

#[test]
fn ts_wildcard_reexport_recorded() {
    let index = build_index(&fixture("reexport_ts")).unwrap();
    // Wildcard re-exports key on the trailing module-name segment.
    let sites = index
        .wildcard_reexports
        .get("widgets")
        .expect("widgets wildcard re-export should be recorded");
    assert_eq!(sites.len(), 1);
    assert!(sites[0].file.ends_with("lib.ts"));
    assert_eq!(sites[0].module_path, "./widgets");
}

#[test]
fn js_alias_reexport_recorded() {
    let index = build_index(&fixture("reexport_js")).unwrap();
    let sites = index
        .alias_reexports
        .get("Baz")
        .expect("Baz alias should be recorded");
    assert_eq!(sites.len(), 1);
    assert!(sites[0].file.ends_with("lib.js"));
    assert_eq!(sites[0].alias, "Baz");
    assert_eq!(sites[0].original, "Bar");
    assert_eq!(sites[0].module_path, "./inner");
}

#[test]
fn js_wildcard_reexport_recorded() {
    let index = build_index(&fixture("reexport_js")).unwrap();
    let sites = index
        .wildcard_reexports
        .get("widgets")
        .expect("widgets wildcard re-export should be recorded");
    assert_eq!(sites.len(), 1);
    assert!(sites[0].file.ends_with("lib.js"));
    assert_eq!(sites[0].module_path, "./widgets");
}

#[test]
fn python_alias_import_treated_as_reexport_site() {
    let index = build_index(&fixture("reexport_py")).unwrap();
    let sites = index
        .alias_reexports
        .get("Baz")
        .expect("Baz alias should be recorded");
    assert_eq!(sites.len(), 1);
    assert!(sites[0].file.ends_with("lib.py"));
    assert_eq!(sites[0].alias, "Baz");
    assert_eq!(sites[0].original, "Bar");
    assert_eq!(sites[0].module_path, "inner");
}

#[test]
fn python_wildcard_import_treated_as_wildcard_reexport_site() {
    let index = build_index(&fixture("reexport_py")).unwrap();
    let sites = index
        .wildcard_reexports
        .get("widgets")
        .expect("widgets wildcard import should be recorded");
    assert_eq!(sites.len(), 1);
    assert!(sites[0].file.ends_with("lib.py"));
    assert_eq!(sites[0].module_path, "widgets");
}

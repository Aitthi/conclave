use codeintel::graph;
use codeintel::index::{build_index, DefKind};
use std::path::Path;

fn ref_kinds(idx: &codeintel::index::Index, name: &str) -> Vec<String> {
    let (data, _) = graph::find_refs(idx, name, 200).unwrap();
    data.as_array()
        .unwrap()
        .iter()
        .map(|h| h["kind"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn go_symbols_and_call_refs() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/go")).unwrap();
    assert!(
        idx.definitions
            .iter()
            .any(|d| d.name == "Greet" && d.file == "util.go"),
        "Greet not found in defs: {:?}",
        idx.definitions
    );
    let kinds = ref_kinds(&idx, "Greet");
    assert!(kinds.iter().any(|k| k == "definition"), "kinds: {kinds:?}");
    assert!(kinds.iter().any(|k| k == "call"), "kinds: {kinds:?}");
}

#[test]
fn go_import_captured_and_cross_file_call_resolves() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/go")).unwrap();
    // Plain `import "example.com/app/util"` has no name node, so index_imports
    // records it as a wildcard binding of the quoted (quote-stripped) path.
    assert!(
        idx.imports.iter().any(|i| i.file == "main.go"
            && i.module_path == "example.com/app/util"
            && i.imported_name == "*"),
        "import not captured: {:?}",
        idx.imports
    );
    // The wildcard import satisfies resolve_refs rule 3: module_matches maps the
    // dotted module-path's last segment onto util.go's file stem.
    let (data, _) = graph::find_refs(&idx, "Greet", 200).unwrap();
    let hits = data.as_array().unwrap();
    assert!(
        hits.iter()
            .any(|h| h["kind"] == "definition" && h["file"] == "util.go"),
        "hits: {hits:?}"
    );
    assert!(
        hits.iter().any(|h| h["kind"] == "call"
            && h["file"] == "main.go"
            && h["reason"] == "import-resolved"),
        "hits: {hits:?}"
    );
}

#[test]
fn c_symbols_and_call_refs() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/c")).unwrap();
    assert!(
        idx.definitions
            .iter()
            .any(|d| d.name == "add" && d.file == "main.c" && d.kind == DefKind::Fn),
        "add not found in defs: {:?}",
        idx.definitions
    );
    let kinds = ref_kinds(&idx, "add");
    assert!(kinds.iter().any(|k| k == "definition"), "kinds: {kinds:?}");
    assert!(kinds.iter().any(|k| k == "call"), "kinds: {kinds:?}");
}

#[test]
fn c_include_captured() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/c")).unwrap();
    assert!(
        idx.imports
            .iter()
            .any(|i| i.file == "main.c" && i.module_path == "util.h"),
        "include not captured: {:?}",
        idx.imports
    );
}

#[test]
fn cpp_class_method_and_call() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/cpp")).unwrap();
    assert!(
        idx.definitions
            .iter()
            .any(|d| d.name == "Shape" && matches!(d.kind, DefKind::Struct | DefKind::Class)),
        "Shape not found in defs: {:?}",
        idx.definitions
    );
    assert!(
        idx.definitions
            .iter()
            .any(|d| d.name == "area" && d.kind == DefKind::Method),
        "area method not found in defs: {:?}",
        idx.definitions
    );
    let kinds = ref_kinds(&idx, "area");
    assert!(kinds.iter().any(|k| k == "definition"), "kinds: {kinds:?}");
    assert!(kinds.iter().any(|k| k == "call"), "kinds: {kinds:?}");
}

#[test]
fn cpp_include_captured() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/cpp")).unwrap();
    assert!(
        idx.imports
            .iter()
            .any(|i| i.file == "shape.cpp" && i.module_path == "shape.hpp"),
        "include not captured: {:?}",
        idx.imports
    );
}

#[test]
fn swift_symbols_and_call_refs() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/swift")).unwrap();
    assert!(
        idx.definitions
            .iter()
            .any(|d| d.name == "greet" && d.file == "main.swift" && d.kind == DefKind::Fn),
        "greet not found in defs: {:?}",
        idx.definitions
    );
    let kinds = ref_kinds(&idx, "greet");
    assert!(kinds.iter().any(|k| k == "definition"), "kinds: {kinds:?}");
    assert!(kinds.iter().any(|k| k == "call"), "kinds: {kinds:?}");
}

#[test]
fn swift_import_captured() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/swift")).unwrap();
    assert!(
        idx.imports
            .iter()
            .any(|i| i.file == "main.swift" && i.module_path == "Foundation"),
        "import not captured: {:?}",
        idx.imports
    );
}

#[test]
fn kotlin_symbols_and_call_refs() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/kotlin")).unwrap();
    assert!(
        idx.definitions
            .iter()
            .any(|d| d.name == "add" && d.file == "App.kt" && d.kind == DefKind::Fn),
        "add not found in defs: {:?}",
        idx.definitions
    );
    let kinds = ref_kinds(&idx, "add");
    assert!(kinds.iter().any(|k| k == "definition"), "kinds: {kinds:?}");
    assert!(kinds.iter().any(|k| k == "call"), "kinds: {kinds:?}");
}

#[test]
fn kotlin_import_captured() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/kotlin")).unwrap();
    assert!(
        idx.imports
            .iter()
            .any(|i| i.file == "App.kt" && i.module_path == "util.help"),
        "import not captured: {:?}",
        idx.imports
    );
}

#[test]
fn java_class_method_and_call() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/java")).unwrap();
    assert!(
        idx.definitions
            .iter()
            .any(|d| d.name == "App" && d.kind == DefKind::Class),
        "App class not found in defs: {:?}",
        idx.definitions
    );
    assert!(
        idx.definitions
            .iter()
            .any(|d| d.name == "add" && d.kind == DefKind::Method),
        "add method not found in defs: {:?}",
        idx.definitions
    );
    let kinds = ref_kinds(&idx, "add");
    assert!(kinds.iter().any(|k| k == "definition"), "kinds: {kinds:?}");
    assert!(kinds.iter().any(|k| k == "call"), "kinds: {kinds:?}");
}

#[test]
fn java_import_captured_and_cross_file_new_resolves() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/java")).unwrap();
    // `import util.Helper;` is a named import: @name binds the class, @path the
    // full dotted path.
    assert!(
        idx.imports.iter().any(|i| i.file == "App.java"
            && i.local_name == "Helper"
            && i.imported_name == "Helper"
            && i.module_path == "util.Helper"),
        "import not captured: {:?}",
        idx.imports
    );
    // `new Helper()` in App.java resolves to the class in util/Helper.java via
    // resolve_refs rule 2: module_matches maps the dotted path's last segment
    // onto the defining file's stem.
    let (data, _) = graph::find_refs(&idx, "Helper", 200).unwrap();
    let hits = data.as_array().unwrap();
    assert!(
        hits.iter()
            .any(|h| h["kind"] == "definition" && h["file"] == "util/Helper.java"),
        "hits: {hits:?}"
    );
    assert!(
        hits.iter().any(|h| h["kind"] == "call"
            && h["file"] == "App.java"
            && h["reason"] == "import-resolved"),
        "hits: {hits:?}"
    );
}

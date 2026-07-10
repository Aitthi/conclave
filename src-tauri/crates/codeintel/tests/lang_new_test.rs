use codeintel::index::{build_index, DefKind};
use codeintel::graph;
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
            .any(|d| d.name == "Greet" && d.file == "main.go"),
        "Greet not found in defs: {:?}",
        idx.definitions
    );
    let kinds = ref_kinds(&idx, "Greet");
    assert!(kinds.iter().any(|k| k == "definition"), "kinds: {kinds:?}");
    assert!(kinds.iter().any(|k| k == "call"), "kinds: {kinds:?}");
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

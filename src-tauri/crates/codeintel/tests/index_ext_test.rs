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

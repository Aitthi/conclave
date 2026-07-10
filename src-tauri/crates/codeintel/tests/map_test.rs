use codeintel::{index::build_index, map};
use std::path::Path;

fn fixture() -> &'static Path {
    Path::new("tests/fixtures/sample_project")
}

// --- files ---

#[test]
fn files_lists_all_supported_extensions() {
    let idx = build_index(fixture()).unwrap();
    let (data, truncated) = map::files(&idx, 200).unwrap();
    assert!(!truncated);
    let paths: Vec<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    for p in [
        "src/lib.rs",
        "src/component.tsx",
        "src/types.ts",
        "src/util.js",
        "app.py",
    ] {
        assert!(paths.contains(&p), "missing {p}");
    }
    let rs = data
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "src/lib.rs")
        .unwrap();
    assert!(rs["lines"].as_u64().unwrap() > 0);
    assert_eq!(rs["language"], "rust");
}

#[test]
fn files_reports_all_expected_languages() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::files(&idx, 200).unwrap();
    let langs: std::collections::HashSet<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["language"].as_str().unwrap())
        .collect();
    for expected in ["rust", "typescript", "tsx", "javascript", "python"] {
        assert!(langs.contains(expected), "missing language {expected}");
    }
}

// --- tree (index-free) ---

#[test]
fn tree_returns_nested_structure() {
    let tree = map::tree(fixture()).unwrap();
    assert!(tree["is_dir"].as_bool().unwrap_or(false));
    let children = tree["children"].as_array().expect("children array");
    let names: Vec<&str> = children.iter().filter_map(|c| c["name"].as_str()).collect();
    assert!(names.contains(&"src"));
    assert!(names.contains(&"app.py"));
}

// --- stats ---

#[test]
fn stats_returns_per_language_and_per_kind() {
    let idx = build_index(fixture()).unwrap();
    let d = map::stats(fixture(), &idx).unwrap();
    assert!(d["total_files"].as_u64().unwrap() >= 5);
    assert!(d["total_lines"].as_u64().unwrap() > 0);
    assert!(d["languages"]["rust"]["files"].as_u64().unwrap() >= 1);
    assert!(d["languages"]["python"]["files"].as_u64().unwrap() >= 1);
    assert!(d["symbols"]["fn"].as_u64().unwrap() >= 1);
    assert!(d["symbols"]["struct"].as_u64().unwrap() >= 1);
    assert!(d["symbols"]["class"].as_u64().unwrap() >= 1);
    assert!(d["symbols"]["interface"].as_u64().unwrap() >= 1);
}

// --- symbols ---

#[test]
fn symbols_rust_single_file() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::symbols(&idx, Some("src/lib.rs"), false, &[], 200).unwrap();
    let mut names: Vec<(String, String)> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_string(),
                s["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    names.sort();
    assert!(names.contains(&("Greeter".into(), "struct".into())));
    assert!(names.contains(&("Mood".into(), "enum".into())));
    assert!(names.contains(&("Speak".into(), "trait".into())));
    assert!(names.contains(&("Result".into(), "type".into())));
    assert!(names.contains(&("VERSION".into(), "const".into())));
    assert!(names.iter().any(|(n, k)| n == "greet" && k == "fn"));
}

#[test]
fn symbols_kind_filter_and_limit_truncate() {
    let idx = build_index(fixture()).unwrap();
    let (all, _) = map::symbols(&idx, None, true, &[], 200).unwrap();
    let n = all.as_array().unwrap().len();
    assert!(n > 2);
    let (cut, truncated) = map::symbols(&idx, None, true, &[], 2).unwrap();
    assert_eq!(cut.as_array().unwrap().len(), 2);
    assert!(truncated);
    let (fns, _) = map::symbols(&idx, None, true, &["fn".into()], 200).unwrap();
    assert!(fns
        .as_array()
        .unwrap()
        .iter()
        .all(|s| s["kind"] == "fn" || s["kind"] == "method"));
}

#[test]
fn symbols_kind_filter_keeps_only_requested() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::symbols(
        &idx,
        Some("src/lib.rs"),
        false,
        &["struct".into(), "enum".into()],
        200,
    )
    .unwrap();
    for s in data.as_array().unwrap() {
        let k = s["kind"].as_str().unwrap();
        assert!(matches!(k, "struct" | "enum"), "unexpected kind {k}");
    }
}

#[test]
fn symbols_typescript_extracts_class_interface_type_fn() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::symbols(&idx, Some("src/types.ts"), false, &[], 200).unwrap();
    let pairs: Vec<(String, String)> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_string(),
                s["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(pairs.contains(&("User".into(), "interface".into())));
    assert!(pairs.contains(&("Status".into(), "type".into())));
    assert!(pairs.contains(&("UserRepo".into(), "class".into())));
    assert!(pairs.contains(&("findUser".into(), "fn".into())));
}

#[test]
fn symbols_tsx_extracts_function_and_interface() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::symbols(&idx, Some("src/component.tsx"), false, &[], 200).unwrap();
    let pairs: Vec<(String, String)> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_string(),
                s["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(pairs.contains(&("Props".into(), "interface".into())));
    assert!(pairs.contains(&("Header".into(), "fn".into())));
}

#[test]
fn symbols_javascript_extracts_function_and_class() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::symbols(&idx, Some("src/util.js"), false, &[], 200).unwrap();
    let pairs: Vec<(String, String)> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_string(),
                s["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(pairs.contains(&("add".into(), "fn".into())));
    assert!(pairs.contains(&("Counter".into(), "class".into())));
}

#[test]
fn symbols_python_extracts_class_and_top_level_fn() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::symbols(&idx, Some("app.py"), false, &[], 200).unwrap();
    let pairs: Vec<(String, String)> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_string(),
                s["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(pairs.contains(&("Cat".into(), "class".into())));
    assert!(pairs.contains(&("main".into(), "fn".into())));
}

#[test]
fn symbols_whole_project_aggregates_all_languages() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::symbols(&idx, Some("."), false, &[], 200).unwrap();
    let files: std::collections::HashSet<String> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["file"].as_str().unwrap().to_string())
        .collect();
    assert!(files.iter().any(|f| f.ends_with("lib.rs")));
    assert!(files.iter().any(|f| f.ends_with("types.ts")));
    assert!(files.iter().any(|f| f.ends_with("component.tsx")));
    assert!(files.iter().any(|f| f.ends_with("util.js")));
    assert!(files.iter().any(|f| f.ends_with("app.py")));
}

// --- find ---

#[test]
fn find_substring_matches_across_languages() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::find(&idx, "User", false, 200).unwrap();
    let names: Vec<&str> = data
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"User"));
    assert!(names.contains(&"UserRepo"));
    assert!(names.contains(&"findUser"));
}

#[test]
fn find_exact_returns_only_exact_name() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::find(&idx, "greet", true, 200).unwrap();
    assert!(data.as_array().unwrap().iter().all(|s| s["name"] == "greet"));
}

#[test]
fn find_exact_only_returns_exact_name_user() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::find(&idx, "User", true, 200).unwrap();
    let arr = data.as_array().unwrap();
    for e in arr {
        assert_eq!(e["name"].as_str().unwrap(), "User");
    }
    assert!(!arr.is_empty());
}

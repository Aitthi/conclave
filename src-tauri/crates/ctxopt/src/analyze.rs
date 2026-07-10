use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::request::{index_tools, ToolCall, ToolResultRef};
use crate::{MIN_ELIDE_BYTES, RECENT_KEEP};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElisionReason {
    IdenticalRead { path: String, kept_msg: usize },
    SupersededRead { path: String, edit_msg: usize },
    DuplicateResult { tool: String, kept_msg: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elision {
    pub tool_use_id: String,
    pub stub: String,
    pub reason: ElisionReason,
}

fn stub_identical(path: &str, kept: usize) -> String {
    format!("[ctxopt] elided: byte-identical to the later Read of {path} (message #{kept}); nothing lost.")
}

fn stub_duplicate(tool: &str, kept: usize) -> String {
    format!("[ctxopt] elided: identical output of the same {tool} call kept at message #{kept}.")
}

fn stub_superseded(path: &str, edit_msg: usize) -> String {
    format!("[ctxopt] elided: this Read of {path} was superseded by a later Edit/Write (message #{edit_msg}); re-read the file for current content.")
}

const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// Path an edit-family tool_use touches (`notebook_path` for NotebookEdit).
fn edited_path(c: &ToolCall) -> Option<&str> {
    if !EDIT_TOOLS.contains(&c.name.as_str()) {
        return None;
    }
    let key = if c.name == "NotebookEdit" { "notebook_path" } else { "file_path" };
    c.input.get(key).and_then(Value::as_str)
}

/// A result may be elided only when its text is known, big enough, and old
/// enough (never within the last RECENT_KEEP messages).
fn elidable(r: &ToolResultRef, total_msgs: usize) -> bool {
    match &r.text {
        Some(t) => t.len() >= MIN_ELIDE_BYTES && r.msg_idx + RECENT_KEEP < total_msgs,
        None => false,
    }
}

pub fn analyze(messages: &Value) -> Vec<Elision> {
    let total_msgs = messages.as_array().map_or(0, Vec::len);
    let (calls, results) = index_tools(messages);
    let call_by_id: HashMap<&str, &ToolCall> = calls.iter().map(|c| (c.id.as_str(), c)).collect();

    // Pair every result with its originating call; results without a matching
    // tool_use are left untouched.
    let paired: Vec<(&ToolResultRef, &ToolCall)> = results
        .iter()
        .filter_map(|r| call_by_id.get(r.tool_use_id.as_str()).map(|c| (r, *c)))
        .collect();

    let mut elisions: Vec<Elision> = Vec::new();
    let mut taken: HashSet<&str> = HashSet::new();
    // Kept copies an IdenticalRead stub points at — their bytes must survive,
    // so the superseded rule never elides them.
    let mut survivors: HashSet<&str> = HashSet::new();

    // Rule 1: identical Reads of one file_path — keep the LAST copy of each
    // byte-identical text, elide earlier ones.
    let mut read_groups: HashMap<(String, &str), Vec<(&ToolResultRef, &ToolCall)>> = HashMap::new();
    for &(r, c) in &paired {
        if c.name != "Read" {
            continue;
        }
        let (Some(path), Some(text)) = (c.input.get("file_path").and_then(Value::as_str), r.text.as_deref()) else { continue };
        read_groups.entry((path.to_string(), text)).or_default().push((r, c));
    }
    for ((path, _), mut group) in read_groups {
        if group.len() < 2 {
            continue;
        }
        group.sort_by_key(|(r, _)| r.msg_idx);
        let kept = group.last().unwrap().0;
        for (r, _) in &group[..group.len() - 1] {
            if elidable(r, total_msgs) && taken.insert(r.tool_use_id.as_str()) {
                survivors.insert(kept.tool_use_id.as_str());
                elisions.push(Elision {
                    tool_use_id: r.tool_use_id.clone(),
                    stub: stub_identical(&path, kept.msg_idx),
                    reason: ElisionReason::IdenticalRead { path: path.clone(), kept_msg: kept.msg_idx },
                });
            }
        }
    }

    // Rule 2: superseded Reads — a later Edit/Write on the same path makes the
    // read provably outdated (re-readable from disk).
    for &(r, c) in &paired {
        if c.name != "Read" || !elidable(r, total_msgs) {
            continue;
        }
        if taken.contains(r.tool_use_id.as_str()) || survivors.contains(r.tool_use_id.as_str()) {
            continue;
        }
        let Some(path) = c.input.get("file_path").and_then(Value::as_str) else { continue };
        let edit_msg = calls
            .iter()
            .filter(|e| e.msg_idx > r.msg_idx && edited_path(e) == Some(path))
            .map(|e| e.msg_idx)
            .min();
        if let Some(edit_msg) = edit_msg {
            taken.insert(r.tool_use_id.as_str());
            elisions.push(Elision {
                tool_use_id: r.tool_use_id.clone(),
                stub: stub_superseded(path, edit_msg),
                reason: ElisionReason::SupersededRead { path: path.to_string(), edit_msg },
            });
        }
    }

    // Rule 3: exact duplicates of any non-Read tool — same tool, same input,
    // byte-identical output: keep the LAST.
    let mut dup_groups: HashMap<(String, String, &str), Vec<(&ToolResultRef, &ToolCall)>> = HashMap::new();
    for &(r, c) in &paired {
        if c.name == "Read" {
            continue;
        }
        let Some(text) = r.text.as_deref() else { continue };
        dup_groups.entry((c.name.clone(), c.input.to_string(), text)).or_default().push((r, c));
    }
    for ((tool, _, _), mut group) in dup_groups {
        if group.len() < 2 {
            continue;
        }
        group.sort_by_key(|(r, _)| r.msg_idx);
        let kept_msg = group.last().unwrap().0.msg_idx;
        for (r, _) in &group[..group.len() - 1] {
            if elidable(r, total_msgs) && taken.insert(r.tool_use_id.as_str()) {
                elisions.push(Elision {
                    tool_use_id: r.tool_use_id.clone(),
                    stub: stub_duplicate(&tool, kept_msg),
                    reason: ElisionReason::DuplicateResult { tool: tool.clone(), kept_msg },
                });
            }
        }
    }

    elisions
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn tool_pair(id: &str, name: &str, input: Value, text: &str) -> [Value; 2] {
        [
            json!({"role": "assistant", "content": [
                {"type": "tool_use", "id": id, "name": name, "input": input}
            ]}),
            json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": id, "content": [{"type": "text", "text": text}]}
            ]}),
        ]
    }

    fn filler(n: usize) -> Vec<Value> {
        (0..n)
            .map(|i| json!({"role": "user", "content": [{"type": "text", "text": format!("filler {i}")}]}))
            .collect()
    }

    fn msgs(pairs: Vec<[Value; 2]>, fill: usize) -> Value {
        let mut out: Vec<Value> = pairs.into_iter().flatten().collect();
        out.extend(filler(fill));
        Value::Array(out)
    }

    #[test]
    fn identical_reads_elide_earlier() {
        let big = "x".repeat(700);
        let m = msgs(
            vec![
                tool_pair("tu_1", "Read", json!({"file_path": "/a.rs"}), &big),
                tool_pair("tu_2", "Read", json!({"file_path": "/a.rs"}), &big),
            ],
            12,
        );
        let e = analyze(&m);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].tool_use_id, "tu_1");
        match &e[0].reason {
            ElisionReason::IdenticalRead { path, kept_msg } => {
                assert_eq!(path, "/a.rs");
                assert_eq!(*kept_msg, 3); // the LATER read's result message
            }
            other => panic!("wrong reason: {other:?}"),
        }
    }

    #[test]
    fn different_bytes_no_elision() {
        let m = msgs(
            vec![
                tool_pair("tu_1", "Read", json!({"file_path": "/a.rs"}), &"x".repeat(700)),
                tool_pair("tu_2", "Read", json!({"file_path": "/a.rs"}), &"y".repeat(700)),
            ],
            12,
        );
        assert!(analyze(&m).is_empty());
    }

    #[test]
    fn recent_messages_never_elided() {
        let big = "x".repeat(700);
        let m = msgs(
            vec![
                tool_pair("tu_1", "Read", json!({"file_path": "/a.rs"}), &big),
                tool_pair("tu_2", "Read", json!({"file_path": "/a.rs"}), &big),
            ],
            0, // everything inside the last 10 messages
        );
        assert!(analyze(&m).is_empty());
    }

    #[test]
    fn duplicate_bash_results_elide_earlier() {
        let big = "b".repeat(700);
        let m = msgs(
            vec![
                tool_pair("tu_1", "Bash", json!({"command": "ls"}), &big),
                tool_pair("tu_2", "Bash", json!({"command": "ls"}), &big),
            ],
            12,
        );
        let e = analyze(&m);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].tool_use_id, "tu_1");
        match &e[0].reason {
            ElisionReason::DuplicateResult { tool, kept_msg } => {
                assert_eq!(tool, "Bash");
                assert_eq!(*kept_msg, 3);
            }
            other => panic!("wrong reason: {other:?}"),
        }
    }

    #[test]
    fn superseded_read_elided() {
        let big = "x".repeat(700);
        let m = msgs(
            vec![
                tool_pair("tu_1", "Read", json!({"file_path": "/a.rs"}), &big),
                tool_pair("tu_2", "Edit", json!({"file_path": "/a.rs", "old_string": "a", "new_string": "b"}), "ok"),
            ],
            12,
        );
        let e = analyze(&m);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].tool_use_id, "tu_1");
        match &e[0].reason {
            ElisionReason::SupersededRead { path, edit_msg } => {
                assert_eq!(path, "/a.rs");
                assert_eq!(*edit_msg, 2); // the Edit's tool_use message
            }
            other => panic!("wrong reason: {other:?}"),
        }
    }

    #[test]
    fn edit_before_read_no_elision() {
        let big = "x".repeat(700);
        let m = msgs(
            vec![
                tool_pair("tu_1", "Edit", json!({"file_path": "/a.rs", "old_string": "a", "new_string": "b"}), "ok"),
                tool_pair("tu_2", "Read", json!({"file_path": "/a.rs"}), &big),
            ],
            12,
        );
        assert!(analyze(&m).is_empty());
    }

    #[test]
    fn edit_on_other_path_no_elision() {
        let big = "x".repeat(700);
        let m = msgs(
            vec![
                tool_pair("tu_1", "Read", json!({"file_path": "/a.rs"}), &big),
                tool_pair("tu_2", "Edit", json!({"file_path": "/b.rs", "old_string": "a", "new_string": "b"}), "ok"),
            ],
            12,
        );
        assert!(analyze(&m).is_empty());
    }

    #[test]
    fn identical_wins_over_superseded() {
        let big = "x".repeat(700);
        let m = msgs(
            vec![
                tool_pair("tu_1", "Read", json!({"file_path": "/a.rs"}), &big),
                tool_pair("tu_2", "Read", json!({"file_path": "/a.rs"}), &big),
                tool_pair("tu_3", "Edit", json!({"file_path": "/a.rs", "old_string": "a", "new_string": "b"}), "ok"),
            ],
            12,
        );
        let e = analyze(&m);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].tool_use_id, "tu_1");
        assert!(matches!(e[0].reason, ElisionReason::IdenticalRead { .. }));
    }

    #[test]
    fn small_results_never_elided() {
        let small = "x".repeat(500);
        let m = msgs(
            vec![
                tool_pair("tu_1", "Read", json!({"file_path": "/a.rs"}), &small),
                tool_pair("tu_2", "Read", json!({"file_path": "/a.rs"}), &small),
            ],
            12,
        );
        assert!(analyze(&m).is_empty());
    }
}

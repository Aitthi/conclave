use std::collections::HashMap;

use serde_json::{json, Value};

use crate::analyze::Elision;

/// Replace the `content` of every elided tool_result with its stub. Only
/// `content` changes — every sibling key (`cache_control`, `is_error`, unknown
/// fields) stays untouched. Returns serialized bytes saved.
pub fn apply(messages: &mut Value, elisions: &[Elision]) -> usize {
    let stubs: HashMap<&str, &str> = elisions.iter().map(|e| (e.tool_use_id.as_str(), e.stub.as_str())).collect();
    let mut saved = 0usize;
    let Some(msgs) = messages.as_array_mut() else { return 0 };
    for msg in msgs {
        let Some(blocks) = msg.get_mut("content").and_then(Value::as_array_mut) else { continue };
        for b in blocks {
            if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(stub) = b.get("tool_use_id").and_then(Value::as_str).and_then(|id| stubs.get(id)) else { continue };
            let replacement = json!([{ "type": "text", "text": stub }]);
            if let Some(obj) = b.as_object_mut() {
                let old_len = obj.get("content").map_or(0, |c| c.to_string().len());
                let new_len = replacement.to_string().len();
                obj.insert("content".into(), replacement);
                saved += old_len.saturating_sub(new_len);
            }
        }
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::{Elision, ElisionReason};
    use crate::validate::validate;
    use serde_json::json;

    fn fixture() -> serde_json::Value {
        let big = "x".repeat(700);
        json!([
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "tu_1", "name": "Read", "input": {"file_path": "/a.rs"}}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "tu_1",
                 "content": [{"type": "text", "text": big}],
                 "cache_control": {"type": "ephemeral"}}
            ]}
        ])
    }

    fn elision() -> Elision {
        Elision {
            tool_use_id: "tu_1".into(),
            stub: "[ctxopt] elided: test stub.".into(),
            reason: ElisionReason::IdenticalRead { path: "/a.rs".into(), kept_msg: 3 },
        }
    }

    #[test]
    fn apply_replaces_content_and_validates() {
        let before = fixture();
        let mut after = before.clone();
        let els = vec![elision()];
        let saved = apply(&mut after, &els);
        assert!(saved > 0);
        let block = &after[1]["content"][0];
        assert_eq!(block["content"][0]["text"], "[ctxopt] elided: test stub.");
        assert_eq!(block["cache_control"]["type"], "ephemeral");
        assert!(validate(&before, &after, &els).is_ok());
    }

    #[test]
    fn validate_rejects_dropped_message() {
        let before = fixture();
        let mut after = before.clone();
        let els = vec![elision()];
        apply(&mut after, &els);
        after.as_array_mut().unwrap().pop();
        let err = validate(&before, &after, &els).unwrap_err();
        assert!(err.contains("length"), "err was: {err}");
    }

    #[test]
    fn validate_rejects_mutated_tool_use() {
        let before = fixture();
        let mut after = before.clone();
        let els = vec![elision()];
        apply(&mut after, &els);
        after[0]["content"][0]["input"]["file_path"] = json!("/evil.rs");
        assert!(validate(&before, &after, &els).is_err());
    }

    #[test]
    fn validate_rejects_grown_content() {
        let before = fixture();
        let mut after = before.clone();
        let els = vec![elision()];
        apply(&mut after, &els);
        after[1]["content"][0]["content"] = json!([{"type": "text", "text": "y".repeat(2000)}]);
        assert!(validate(&before, &after, &els).is_err());
    }

    #[test]
    fn empty_elisions_noop() {
        let before = fixture();
        let mut after = before.clone();
        assert_eq!(apply(&mut after, &[]), 0);
        assert!(validate(&before, &before, &[]).is_ok());
    }
}

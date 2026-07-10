use serde_json::Value;

pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub msg_idx: usize,
}

pub struct ToolResultRef {
    pub tool_use_id: String,
    pub msg_idx: usize,
    pub block_idx: usize,
    pub text: Option<String>,
}

pub fn index_tools(messages: &Value) -> (Vec<ToolCall>, Vec<ToolResultRef>) {
    let (mut calls, mut results) = (Vec::new(), Vec::new());
    let Some(msgs) = messages.as_array() else { return (calls, results) };
    for (mi, msg) in msgs.iter().enumerate() {
        let Some(blocks) = msg.get("content").and_then(Value::as_array) else { continue };
        for (bi, b) in blocks.iter().enumerate() {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let (Some(id), Some(name)) = (b.get("id").and_then(Value::as_str), b.get("name").and_then(Value::as_str)) else { continue };
                    calls.push(ToolCall { id: id.into(), name: name.into(), input: b.get("input").cloned().unwrap_or(Value::Null), msg_idx: mi });
                }
                Some("tool_result") => {
                    let Some(id) = b.get("tool_use_id").and_then(Value::as_str) else { continue };
                    results.push(ToolResultRef { tool_use_id: id.into(), msg_idx: mi, block_idx: bi, text: result_text(b) });
                }
                _ => {}
            }
        }
    }
    (calls, results)
}

/// Some iff content is a string or all-text blocks.
pub fn result_text(block: &Value) -> Option<String> {
    match block.get("content") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(parts)) => {
            let mut out = String::new();
            for p in parts {
                if p.get("type").and_then(Value::as_str) != Some("text") {
                    return None;
                }
                out.push_str(p.get("text")?.as_str()?);
            }
            Some(out)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> serde_json::Value {
        json!([
            {
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "tu_1", "name": "Read", "input": {"file_path": "/a.rs"}},
                    {"type": "tool_use", "id": "tu_2", "name": "Bash", "input": {"command": "ls"}}
                ]
            },
            {
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "tu_1", "content": [{"type": "text", "text": "fn main(){}"}]},
                    {"type": "tool_result", "tool_use_id": "tu_2", "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}}]}
                ]
            }
        ])
    }

    #[test]
    fn indexes_pairs_and_extracts_text() {
        let msgs = fixture();
        let (calls, results) = index_tools(&msgs);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "Read");
        assert_eq!(results[0].tool_use_id, "tu_1");
        assert_eq!(results[0].text.as_deref(), Some("fn main(){}"));
        assert_eq!(results[1].text, None); // image content → not elidable
    }
}

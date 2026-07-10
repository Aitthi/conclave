use std::collections::HashSet;

use serde_json::Value;

use crate::analyze::Elision;

fn block_type(b: &Value) -> &str {
    b.get("type").and_then(Value::as_str).unwrap_or("")
}

/// Structural proof that `after` differs from `before` ONLY by shrinking the
/// `content` of elided tool_results. Any other difference is an `Err`.
pub fn validate(before: &Value, after: &Value, elisions: &[Elision]) -> Result<(), String> {
    let elided: HashSet<&str> = elisions.iter().map(|e| e.tool_use_id.as_str()).collect();

    let (Some(b_msgs), Some(a_msgs)) = (before.as_array(), after.as_array()) else {
        return Err("messages not arrays".into());
    };
    if b_msgs.len() != a_msgs.len() {
        return Err(format!("message length changed: {} -> {}", b_msgs.len(), a_msgs.len()));
    }

    for (mi, (bm, am)) in b_msgs.iter().zip(a_msgs).enumerate() {
        if bm.get("role") != am.get("role") {
            return Err(format!("message #{mi}: role changed"));
        }
        // Non-array content (plain string) must be byte-equal — nothing to elide there.
        match (bm.get("content").and_then(Value::as_array), am.get("content").and_then(Value::as_array)) {
            (Some(bb), Some(ab)) => {
                if bb.len() != ab.len() {
                    return Err(format!("message #{mi}: block count length changed"));
                }
                for (bi, (bblk, ablk)) in bb.iter().zip(ab).enumerate() {
                    let bt = block_type(bblk);
                    if bt != block_type(ablk) {
                        return Err(format!("message #{mi} block #{bi}: type changed"));
                    }
                    if bt != "tool_result" {
                        if bblk != ablk {
                            return Err(format!("message #{mi} block #{bi}: non-tool_result block changed"));
                        }
                        continue;
                    }
                    let id = bblk.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                    if ablk.get("tool_use_id").and_then(Value::as_str).unwrap_or("") != id {
                        return Err(format!("message #{mi} block #{bi}: tool_use_id changed"));
                    }
                    if !elided.contains(id) {
                        if bblk != ablk {
                            return Err(format!("message #{mi} block #{bi}: non-elided tool_result changed"));
                        }
                        continue;
                    }
                    // Elided: content strictly smaller, all other keys equal.
                    let b_content = bblk.get("content").map_or(0, |c| c.to_string().len());
                    let a_content = ablk.get("content").map_or(0, |c| c.to_string().len());
                    if a_content >= b_content {
                        return Err(format!("message #{mi} block #{bi}: elided content did not shrink"));
                    }
                    let (Some(bo), Some(ao)) = (bblk.as_object(), ablk.as_object()) else {
                        return Err(format!("message #{mi} block #{bi}: block not an object"));
                    };
                    let b_keys: HashSet<&String> = bo.keys().collect();
                    let a_keys: HashSet<&String> = ao.keys().collect();
                    if b_keys != a_keys {
                        return Err(format!("message #{mi} block #{bi}: key set changed"));
                    }
                    for (k, bv) in bo {
                        if k != "content" && ao.get(k) != Some(bv) {
                            return Err(format!("message #{mi} block #{bi}: sibling key {k} changed"));
                        }
                    }
                }
            }
            _ => {
                if bm != am {
                    return Err(format!("message #{mi}: non-block message changed"));
                }
            }
        }
    }
    Ok(())
}

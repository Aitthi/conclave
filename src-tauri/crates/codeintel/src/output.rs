use serde_json::{json, Value};

/// Wrap a command payload in the stable wire envelope.
pub fn envelope(data: Value) -> Value {
    json!({ "schema_version": 1, "data": data })
}

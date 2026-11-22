//! Model-only lossless references; the checkpoint ledger remains unchanged.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) fn project(ledger: &str, last_observation: &str) -> String {
    let last = last_observation.trim();
    if last.chars().count() < 512 {
        return ledger.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<Value>(ledger) else {
        return ledger.to_string();
    };
    let rows = if value.is_array() {
        value.as_array_mut()
    } else {
        value
            .get_mut("recent_attempts")
            .and_then(Value::as_array_mut)
    };
    let Some(row) = rows
        .and_then(|rows| rows.last_mut())
        .and_then(Value::as_object_mut)
    else {
        return ledger.to_string();
    };
    if row.contains_key("observed_output_reference") {
        return ledger.to_string();
    }
    let Some(excerpt) = row.get("observed_output").and_then(Value::as_str) else {
        return ledger.to_string();
    };
    if excerpt.chars().count() < 512 {
        return ledger.to_string();
    }
    let projection = if excerpt == last {
        "identity"
    } else if excerpt == crate::truncate_for_agent_trace(last) {
        "agent_trace_v1"
    } else {
        return ledger.to_string();
    };
    let reference = json!({"schema_version":1,"source":"LAST_TOOL_OR_MODEL_OUTPUT",
        "projection":projection,"scope":"value_excerpt_only",
        "excerpt_chars":excerpt.chars().count(),
        "excerpt_sha256":format!("sha256:{:x}",Sha256::digest(excerpt.as_bytes()))});
    row.remove("observed_output");
    row.insert("observed_output_reference".into(), reference);
    value.to_string()
}

#[cfg(test)]
#[path = "native_ledger_projection_tests.rs"]
mod tests;

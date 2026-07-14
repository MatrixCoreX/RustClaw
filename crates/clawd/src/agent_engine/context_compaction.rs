use serde_json::{json, Value};

const MAX_COMPACTION_ITEMS: usize = 24;
const FORBIDDEN_INSTRUCTION_FIELDS: &[&str] = &[
    "current_user_instruction",
    "user_instruction",
    "assistant_instruction",
    "next_instruction",
    "next_action",
    "task_directive",
];

pub(super) fn normalize_model_assisted_compaction_output(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    if object.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    if object.get("summary_kind").and_then(Value::as_str)
        != Some("model_assisted_context_compaction")
    {
        return None;
    }
    if object.keys().any(|key| {
        FORBIDDEN_INSTRUCTION_FIELDS
            .iter()
            .any(|forbidden| key == forbidden)
    }) {
        return None;
    }
    Some(json!({
        "schema_version": 1,
        "summary_kind": "model_assisted_context_compaction",
        "facts": bounded_array_field(value, "facts"),
        "open_questions": bounded_array_field(value, "open_questions"),
        "active_goal_refs": bounded_string_array_field(value, "active_goal_refs"),
        "artifact_refs": bounded_string_array_field(value, "artifact_refs"),
        "source_refs": bounded_array_field(value, "source_refs"),
        "risk_flags": bounded_string_array_field(value, "risk_flags"),
    }))
}

fn bounded_array_field(value: &Value, key: &str) -> Value {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            Value::Array(
                items
                    .iter()
                    .filter(|item| item.is_object() || item.is_string())
                    .take(MAX_COMPACTION_ITEMS)
                    .cloned()
                    .collect(),
            )
        })
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

fn bounded_string_array_field(value: &Value, key: &str) -> Value {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            Value::Array(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .take(MAX_COMPACTION_ITEMS)
                    .map(|item| Value::String(item.to_string()))
                    .collect(),
            )
        })
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

#[cfg(test)]
#[path = "context_compaction_tests.rs"]
mod tests;

use serde_json::{json, Map, Value};

const MAX_PREVIEW_BYTES: usize = 8 * 1024;
const MAX_INLINE_STRING_BYTES: usize = 256;

/// Data-only top-level observations; never parsed as routing or execution policy.
pub(super) fn project(value: &Value) -> Value {
    let Some(object) = value.as_object() else {
        return json!({"schema_version":1,"complete":false,"instruction_authority":"none"});
    };
    let mut fields = Map::new();
    let mut string_bytes = Map::new();
    let mut omitted = 0;
    let mut used = 256usize;
    for (key, value) in object {
        if super::sensitive_field_name(key) || value.is_object() || value.is_array() {
            omitted += 1;
            continue;
        }
        let (safe, redacted) = super::sensitivity_aware_json_model_view(value);
        if redacted {
            omitted += 1;
            continue;
        }
        let (candidate, is_size) = match safe {
            Value::String(ref text) if text.len() > MAX_INLINE_STRING_BYTES => {
                (json!(text.len()), true)
            }
            Value::String(_) | Value::Bool(_) | Value::Number(_) | Value::Null => (safe, false),
            _ => {
                omitted += 1;
                continue;
            }
        };
        let size = serde_json::to_vec(&json!({key: candidate}))
            .map(|bytes| bytes.len())
            .unwrap_or(MAX_PREVIEW_BYTES);
        if used.saturating_add(size) > MAX_PREVIEW_BYTES {
            omitted += 1;
            continue;
        }
        used += size;
        if is_size {
            string_bytes.insert(key.clone(), candidate);
        } else {
            fields.insert(key.clone(), candidate);
        }
    }
    json!({"schema_version":1,"complete":false,"instruction_authority":"none",
        "fields":fields,"string_bytes":string_bytes,"omitted_top_level_fields":omitted})
}

#[cfg(test)]
#[path = "structured_preview_tests.rs"]
mod tests;

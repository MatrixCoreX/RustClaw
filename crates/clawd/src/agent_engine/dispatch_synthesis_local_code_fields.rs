use serde_json::Value;

pub(super) fn local_code_json_projection_field_value_supported(field: &str, value: &Value) -> bool {
    if field != "error_codes" {
        return true;
    }
    let values = string_or_array_values(value);
    !values.is_empty()
        && values
            .iter()
            .all(|value| machine_error_code_token(value.as_str()))
}

pub(super) fn machine_error_code_token(value: &str) -> bool {
    let value = value.trim();
    if !super::machine_code_token(value) {
        return false;
    }
    !matches!(
        value.to_ascii_lowercase().as_str(),
        "error_code" | "ok" | "value" | "true" | "false" | "none" | "null"
    )
}

fn string_or_array_values(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => vec![text.trim().to_string()],
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

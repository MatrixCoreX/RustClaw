pub(super) fn scalar_answer_from_json(value: &serde_json::Value) -> Option<String> {
    if let Some(answer) = value.get("extra").and_then(scalar_answer_from_json) {
        return Some(answer);
    }
    if let Some(answer) = scalar_answer_from_counts(value) {
        return Some(answer);
    }
    for key in ["value_text", "value", "count", "total", "schema_version"] {
        let Some(child) = value.get(key) else {
            continue;
        };
        if let Some(raw) = child.as_str() {
            let text = raw.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
            if key == "value" {
                return serde_json::to_string(raw).ok();
            }
        }
        if child.is_number() || child.is_boolean() {
            return Some(child.to_string());
        }
    }
    None
}

fn scalar_answer_from_counts(value: &serde_json::Value) -> Option<String> {
    let counts = value.get("counts")?;
    let count_key = scalar_count_key_from_inventory_flags(value);
    counts
        .get(count_key)
        .or_else(|| counts.get("total"))
        .and_then(scalar_json_value_text)
}

fn scalar_count_key_from_inventory_flags(value: &serde_json::Value) -> &'static str {
    let kind_filter = value
        .get("kind_filter")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase());
    if value
        .get("files_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || matches!(
            kind_filter.as_deref(),
            Some("file" | "files" | "regular_file")
        )
    {
        "files"
    } else if value
        .get("dirs_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || matches!(
            kind_filter.as_deref(),
            Some("dir" | "dirs" | "directory" | "directories")
        )
    {
        "dirs"
    } else {
        "total"
    }
}

fn scalar_json_value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "loop_reply_scalar_answer_tests.rs"]
mod tests;

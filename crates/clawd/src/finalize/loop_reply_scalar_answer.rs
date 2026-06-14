pub(super) fn scalar_answer_from_json(value: &serde_json::Value) -> Option<String> {
    if let Some(answer) = value.get("extra").and_then(scalar_answer_from_json) {
        return Some(answer);
    }
    if let Some(answer) = value
        .get("text")
        .and_then(|value| value.as_str())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|value| scalar_answer_from_json(&value))
    {
        return Some(answer);
    }
    for key in ["value_text", "value", "count", "total"] {
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

#[cfg(test)]
mod tests {
    use super::scalar_answer_from_json;

    #[test]
    fn empty_string_field_value_returns_json_string_literal() {
        let value = serde_json::json!({
            "action": "read_field",
            "exists": true,
            "field_path": "workspace.package.repository",
            "value": "",
            "value_text": "",
            "value_type": "string"
        });

        assert_eq!(scalar_answer_from_json(&value).as_deref(), Some("\"\""));
    }

    #[test]
    fn wrapped_empty_string_field_value_returns_json_string_literal() {
        let value = serde_json::json!({
            "extra": {
                "action": "read_field",
                "exists": true,
                "field_path": "workspace.package.repository",
                "value": "",
                "value_text": "",
                "value_type": "string"
            },
            "text": "{\"action\":\"read_field\",\"exists\":true,\"value\":\"\",\"value_text\":\"\"}"
        });

        assert_eq!(scalar_answer_from_json(&value).as_deref(), Some("\"\""));
    }

    #[test]
    fn missing_null_field_value_stays_without_scalar_answer() {
        let value = serde_json::json!({
            "action": "read_field",
            "exists": false,
            "field_path": "package.name",
            "value": null,
            "value_text": "",
            "value_type": "null"
        });

        assert_eq!(scalar_answer_from_json(&value), None);
    }
}

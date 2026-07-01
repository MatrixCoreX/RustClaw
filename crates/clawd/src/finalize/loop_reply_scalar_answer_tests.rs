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
        "text": "{\"action\":\"read_field\",\"exists\":true,\"value\":\"hidden\",\"value_text\":\"hidden\"}"
    });

    assert_eq!(scalar_answer_from_json(&value).as_deref(), Some("\"\""));
}

#[test]
fn scalar_answer_ignores_json_hidden_in_visible_text() {
    let value = serde_json::json!({
        "text": "{\"action\":\"read_field\",\"exists\":true,\"value\":\"hidden\",\"value_text\":\"hidden\"}"
    });

    assert_eq!(scalar_answer_from_json(&value), None);
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

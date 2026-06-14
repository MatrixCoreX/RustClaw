use serde_json::json;

#[test]
fn extract_field_empty_string_keeps_visible_machine_scalar() {
    let value = json!({
        "action": "read_field",
        "exists": true,
        "field_path": "workspace.package.repository",
        "resolved_field_path": "workspace.package.repository",
        "path": "Cargo.toml",
        "resolved_path": "Cargo.toml",
        "value": "",
        "value_text": "",
        "value_type": "string"
    });

    let observation = super::structured_scalar_observation_from_extract_item(&value, None)
        .expect("empty string field value should remain a structured scalar");

    assert_eq!(observation.text, "\"\"");
}

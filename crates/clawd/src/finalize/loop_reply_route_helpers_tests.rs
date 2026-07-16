use super::structured_json_values_from_output;

#[test]
fn structured_json_values_from_output_ignores_visible_text_json_payload() {
    let values = structured_json_values_from_output(
        r#"{"status":"ok","extra":{"action":"find_path","count":1},"text":"{\"action\":\"find_path\",\"count\":99}"}"#,
    );

    assert_eq!(values.len(), 2);
    assert!(values.iter().any(|value| value.get("extra").is_some()));
    assert!(values
        .iter()
        .any(|value| value.get("count") == Some(&serde_json::json!(1))));
    assert!(!values
        .iter()
        .any(|value| value.get("count") == Some(&serde_json::json!(99))));
}

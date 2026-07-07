#[test]
fn normalized_success_body_uses_machine_extra_transform_payload() {
    let raw = serde_json::json!({
        "status": "ok",
        "text": "formatted output",
        "extra": {
            "status": "ok",
            "formatted": "done"
        }
    })
    .to_string();

    assert_eq!(
        super::normalized_success_body_for_observed_output(&raw),
        r#"{"formatted":"done","status":"ok"}"#
    );
}

#[test]
fn normalized_success_body_uses_machine_extra_direct_observation_payload() {
    let raw = serde_json::json!({
        "status": "ok",
        "text": "field value",
        "extra": {
            "action": "read_field",
            "field_value": "v1"
        }
    })
    .to_string();

    assert_eq!(
        super::normalized_success_body_for_observed_output(&raw),
        r#"{"action":"read_field","field_value":"v1"}"#
    );
}

#[test]
fn normalized_success_body_ignores_json_hidden_in_visible_text() {
    let hidden_payload = serde_json::json!({
        "action": "read_field",
        "field_value": "v1"
    })
    .to_string();
    let raw = serde_json::json!({
        "status": "ok",
        "text": hidden_payload
    })
    .to_string();

    assert_eq!(
        super::normalized_success_body_for_observed_output(&raw),
        raw
    );
}

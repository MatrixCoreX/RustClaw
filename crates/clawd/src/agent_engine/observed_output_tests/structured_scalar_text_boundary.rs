#[test]
fn structured_scalar_observation_ignores_visible_text_json_payload() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        &serde_json::json!({
            "status": "ok",
            "text": serde_json::json!({
                "action": "extract_field",
                "exists": true,
                "field_path": "name",
                "value_text": "react-example",
                "value": "react-example",
                "value_type": "string"
            })
            .to_string()
        })
        .to_string(),
    ));

    assert!(super::latest_structured_scalar_observation_text(&loop_state).is_none());
}

#[test]
fn structured_scalar_observation_reads_extra_payload() {
    let mut loop_state = LoopState::new();
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        &serde_json::json!({
            "status": "ok",
            "extra": {
                "action": "extract_field",
                "exists": true,
                "field_path": "name",
                "value_text": "react-example",
                "value": "react-example",
                "value_type": "string"
            }
        })
        .to_string(),
    ));

    assert_eq!(
        super::latest_structured_scalar_observation_text(&loop_state).as_deref(),
        Some("react-example")
    );
}

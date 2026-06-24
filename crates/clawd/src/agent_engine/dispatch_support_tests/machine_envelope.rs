use crate::agent_engine::LoopState;

#[test]
fn machine_json_respond_envelope_publishes_even_when_matching_last_output() {
    let envelope = serde_json::json!({
        "output_format": "machine_json",
        "owner_layer": "subagent_boundary_review",
        "boundary": {
            "write_enabled": false,
            "external_publish_enabled": false
        }
    })
    .to_string();
    let mut loop_state = LoopState::default();
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some(envelope.clone());

    assert!(super::super::should_publish_respond_message(
        &loop_state,
        &envelope
    ));
}

#[test]
fn non_machine_respond_matching_last_output_stays_trace_only() {
    let mut loop_state = LoopState::default();
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some("{\"status\":\"ok\"}".to_string());

    assert!(!super::super::should_publish_respond_message(
        &loop_state,
        "{\"status\":\"ok\"}"
    ));
}

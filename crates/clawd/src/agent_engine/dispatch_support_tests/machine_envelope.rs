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
fn lifecycle_result_respond_payload_publishes_even_when_matching_last_output() {
    let payload = serde_json::json!({
        "final_answer_shape": "lifecycle_result",
        "final_answer_shape_class": "verdict",
        "status": "ok",
        "observed_action_count": 5,
        "observed_actions": [
            "make_dir",
            "write_text",
            "append_text",
            "read_range",
            "remove_path"
        ],
        "steps": [
            {"step_id": "step_1", "skill": "fs_basic", "status": "ok", "action": "make_dir"}
        ],
        "final_state": {"cleanup_observed": true}
    })
    .to_string();
    let mut loop_state = LoopState::default();
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_output = Some(payload.clone());

    assert!(super::super::should_publish_respond_message(
        &loop_state,
        &payload
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

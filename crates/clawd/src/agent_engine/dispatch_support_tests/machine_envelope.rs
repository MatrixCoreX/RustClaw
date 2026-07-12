use super::test_state_with_registry;
use crate::agent_engine::support::load_agent_loop_guard_policy;
use crate::agent_engine::LoopState;
use crate::AgentAction;

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

#[test]
fn terminal_last_output_placeholder_respond_publishes_structured_output() {
    let state = test_state_with_registry();
    let task = crate::ClaimedTask {
        task_id: "task-terminal-last-output-placeholder-respond".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: String::new(),
    };
    let policy = load_agent_loop_guard_policy(&state);
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.has_tool_or_skill_output = true;
    let content = r#"{"changed_files":["calc_core.py","test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"OK","functions":["add","sub","mul"]}"#;
    loop_state.last_output = Some(content.to_string());
    loop_state
        .output_vars
        .insert("last_output".to_string(), content.to_string());
    let actions = vec![AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    }];

    let outcome = super::super::handle_respond_action(
        &state,
        &task,
        &actions,
        &mut loop_state,
        &policy,
        0,
        1,
        1,
        "respond:terminal_last_output",
        "{{last_output}}",
        None,
    );

    assert!(outcome.should_stop);
    assert_eq!(outcome.stop_signal.as_deref(), Some("respond"));
    assert!(outcome.ended_with_user_visible_output);
    assert_eq!(loop_state.delivery_messages, vec![content.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(content)
    );
}

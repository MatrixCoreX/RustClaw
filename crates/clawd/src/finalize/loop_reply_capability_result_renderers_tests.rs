use super::*;

#[test]
fn capability_result_renderer_dispatch_records_structured_trace_when_skipped() {
    let state = test_state();
    let task = claimed_task("task-capability-result-renderer-trace");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    let mut finalizer_summary = None;

    let rendered = attach_config_edit_observed_answer_from_registry(
        &state,
        &task,
        "config edit observation",
        &mut loop_state,
        None,
        &mut finalizer_summary,
    );

    assert!(!rendered);
    let trace = loop_state
        .output_vars
        .get("finalizer.renderer_trace.config_edit_observed_answer")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .expect("renderer trace output var");
    assert_eq!(trace["kind"], "finalizer_renderer_trace");
    assert_eq!(trace["renderer_key"], "config_edit_observed_answer");
    assert_eq!(trace["shape"], "capability_result");
    assert_eq!(trace["disposition"], "skipped");
    assert_eq!(trace["failure_reason"], "not_applicable");
    assert_eq!(
        trace["evidence_refs"]
            .as_array()
            .and_then(|refs| refs.first())
            .and_then(serde_json::Value::as_str),
        Some("task:task-capability-result-renderer-trace")
    );
}

#[test]
fn config_edit_renderer_replaces_machine_marker_delivery_with_structured_evidence() {
    let state = test_state();
    let task = claimed_task("task-config-edit-marker-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"read_field","exists":true,"field_path":"llm.selected_vendor","format":"toml","path":"/home/guagua/rustclaw/configs/config.toml","resolved_field_path":"llm.selected_vendor","resolved_path":"/home/guagua/rustclaw/configs/config.toml","value":"minimax","value_text":"minimax","value_type":"string"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"extra":{"action":"guard_config","candidates":["tools.allow_sudo=true"],"count":1,"format":"toml","path":"/home/guagua/rustclaw/configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","risk_count":1,"risks":["tools.allow_sudo=true"],"valid":false}}"#,
    ));
    let mut finalizer_summary = None;

    let rendered = attach_config_edit_observed_answer_from_registry(
        &state,
        &task,
        "preview config edit",
        &mut loop_state,
        None,
        &mut finalizer_summary,
    );

    assert!(rendered);
    assert_eq!(loop_state.delivery_messages.len(), 1);
    let payload: serde_json::Value =
        serde_json::from_str(&loop_state.delivery_messages[0]).expect("structured payload");
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_preview_read_guard")
    );
    assert_eq!(
        payload
            .pointer("/current_value")
            .and_then(serde_json::Value::as_str),
        Some("minimax")
    );
    assert_eq!(
        payload
            .pointer("/risk_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert!(finalizer_summary.is_some());
}

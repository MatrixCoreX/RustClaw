use super::*;
use crate::finalize::loop_reply::replace_delivery_with_direct_scalar_observed_answer;

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
        Some("config_edit_read_guard")
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

#[test]
fn config_edit_renderer_accepts_config_capability_step_names() {
    let state = test_state();
    let task = claimed_task("task-config-edit-capability-step-names");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config.read_field",
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"llm.selected_vendor","format":"toml","match_count":1,"match_strategy":"exact_path","path":"configs/config.toml","resolved_field_path":"llm.selected_vendor","resolved_path":"/home/guagua/rustclaw/configs/config.toml","value":"minimax","value_text":"minimax","value_type":"string"},"text":"{\"action\":\"extract_field\",\"exists\":true,\"field_path\":\"llm.selected_vendor\",\"format\":\"toml\",\"match_count\":1,\"match_strategy\":\"exact_path\",\"path\":\"configs/config.toml\",\"resolved_field_path\":\"llm.selected_vendor\",\"resolved_path\":\"/home/guagua/rustclaw/configs/config.toml\",\"value\":\"minimax\",\"value_text\":\"minimax\",\"value_type\":\"string\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config.guard_config",
        r#"{"extra":{"action":"guard_config","candidates":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"count":2,"format":"toml","path":"configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"valid":false},"text":"{\"action\":\"guard_config\",\"candidates\":[\"tools.allow_sudo=true\",\"tools.allow_path_outside_workspace=true\"],\"count\":2,\"format\":\"toml\",\"path\":\"configs/config.toml\",\"resolved_path\":\"/home/guagua/rustclaw/configs/config.toml\",\"risk_count\":2,\"risks\":[\"tools.allow_sudo=true\",\"tools.allow_path_outside_workspace=true\"],\"valid\":false}"}"#,
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
    let payload: serde_json::Value =
        serde_json::from_str(&loop_state.delivery_messages[0]).expect("structured payload");
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_read_guard")
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
        Some(2)
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn config_edit_renderer_accepts_resolved_system_basic_config_observations() {
    let state = test_state();
    let task = claimed_task("task-config-edit-system-basic-observations");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "system_basic",
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"llm.selected_vendor","format":"toml","match_count":1,"match_strategy":"exact_path","path":"configs/config.toml","resolved_field_path":"llm.selected_vendor","resolved_path":"/home/guagua/rustclaw/configs/config.toml","value":"minimax","value_text":"minimax","value_type":"string"},"text":"{\"action\":\"extract_field\",\"exists\":true,\"field_path\":\"llm.selected_vendor\",\"format\":\"toml\",\"match_count\":1,\"match_strategy\":\"exact_path\",\"path\":\"configs/config.toml\",\"resolved_field_path\":\"llm.selected_vendor\",\"resolved_path\":\"/home/guagua/rustclaw/configs/config.toml\",\"value\":\"minimax\",\"value_text\":\"minimax\",\"value_type\":\"string\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "system_basic",
        r#"{"extra":{"action":"guard_config","candidates":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"count":2,"format":"toml","path":"configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"valid":false},"text":"{\"action\":\"guard_config\",\"candidates\":[\"tools.allow_sudo=true\",\"tools.allow_path_outside_workspace=true\"],\"count\":2,\"format\":\"toml\",\"path\":\"configs/config.toml\",\"resolved_path\":\"/home/guagua/rustclaw/configs/config.toml\",\"risk_count\":2,\"risks\":[\"tools.allow_sudo=true\",\"tools.allow_path_outside_workspace=true\"],\"valid\":false}"}"#,
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
    let payload: serde_json::Value =
        serde_json::from_str(&loop_state.delivery_messages[0]).expect("structured payload");
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_read_guard")
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
        Some(2)
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn config_edit_renderer_preserves_terminal_machine_payload() {
    let state = test_state();
    let task = claimed_task("task-config-edit-terminal-machine-payload");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    let payload = r#"{"candidates":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"count":2,"current_value":"minimax","field_path":"llm.selected_vendor","message_key":"clawd.msg.config_edit.read_guard","path":"configs/config.toml","reason_code":"config_edit_read_guard","risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"]}"#;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", payload));
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
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&loop_state.delivery_messages[0]).unwrap(),
        serde_json::from_str::<serde_json::Value>(payload).unwrap()
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn config_edit_renderer_preserves_wrapped_terminal_machine_payload() {
    let state = test_state();
    let task = claimed_task("task-config-edit-wrapped-terminal-machine-payload");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    let payload = r#"{"field_path":"llm.selected_vendor","message_key":"clawd.msg.config_edit.read_guard","path":"configs/config.toml","reason_code":"config_edit_read_guard","risk_count":2,"current_value":"minimax"}"#;
    let wrapped = serde_json::json!({ "text": payload }).to_string();
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", &wrapped));
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
    let actual: serde_json::Value =
        serde_json::from_str(&loop_state.delivery_messages[0]).expect("structured payload");
    assert_eq!(
        actual
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_read_guard")
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn config_edit_renderer_replaces_visible_marker_inside_non_single_delivery() {
    let state = test_state();
    let task = claimed_task("task-config-edit-visible-marker-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec![
        r#"{"message_key":"clawd.msg.execution.summary","step_count":3}"#.to_string(),
        "llm.selected_vendor".to_string(),
    ];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    let payload = r#"{"field_path":"llm.selected_vendor","message_key":"clawd.msg.config_edit.read_guard","path":"configs/config.toml","reason_code":"config_edit_read_guard","risk_count":2,"current_value":"minimax"}"#;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", payload));
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
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&loop_state.delivery_messages[0]).unwrap(),
        serde_json::from_str::<serde_json::Value>(payload).unwrap()
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn final_config_edit_renderer_replaces_visible_marker_inside_non_single_delivery() {
    let state = test_state();
    let task = claimed_task("task-config-edit-final-visible-marker-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec![
        r#"{"message_key":"clawd.msg.execution.summary","step_count":3}"#.to_string(),
        "llm.selected_vendor".to_string(),
    ];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    let payload = r#"{"field_path":"llm.selected_vendor","message_key":"clawd.msg.config_edit.read_guard","path":"configs/config.toml","reason_code":"config_edit_read_guard","risk_count":2,"current_value":"minimax"}"#;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", payload));
    let mut finalizer_summary = None;
    let mut delivery_deduped = loop_state.delivery_messages.clone();

    let rendered = replace_config_edit_machine_marker_delivery(
        &state,
        &task,
        "preview config edit",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_deduped,
    );

    assert!(rendered);
    assert_eq!(delivery_deduped.len(), 1);
    assert_eq!(loop_state.delivery_messages.len(), 1);
    let expected_payload = serde_json::from_str::<serde_json::Value>(payload).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&delivery_deduped[0]).unwrap(),
        expected_payload
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&loop_state.delivery_messages[0]).unwrap(),
        expected_payload
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn config_edit_renderer_skips_later_respond_marker_for_terminal_machine_payload() {
    let state = test_state();
    let task = claimed_task("task-config-edit-terminal-machine-payload-before-marker");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    let payload = r#"{"candidates":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"count":2,"current_value":"minimax","field_path":"llm.selected_vendor","message_key":"clawd.msg.config_edit.read_guard","path":"configs/config.toml","reason_code":"config_edit_read_guard","risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"]}"#;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", payload));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "respond",
        "llm.selected_vendor",
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
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&loop_state.delivery_messages[0]).unwrap(),
        serde_json::from_str::<serde_json::Value>(payload).unwrap()
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn config_edit_renderer_preserves_live_preview_synthesis_payload() {
    let state = test_state();
    let task = claimed_task("task-config-edit-live-preview-synthesis-payload");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_edit",
        r#"{"extra":{"action":"plan_config_change","exists":true,"field_path":"llm.selected_vendor","field_value":{"exists":true,"field_path":"llm.selected_vendor","new_value":"minimax","old_value":"minimax","would_change":false},"format":"toml","new_value":"minimax","old_value":"minimax","operation":"set","path":"/home/guagua/rustclaw/configs/config.toml","requires_confirmation":true,"resolved_path":"/home/guagua/rustclaw/configs/config.toml","restart_recommended":true,"would_change":false}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"llm.selected_vendor","format":"toml","match_count":1,"match_strategy":"exact_path","path":"/home/guagua/rustclaw/configs/config.toml","resolved_field_path":"llm.selected_vendor","resolved_path":"/home/guagua/rustclaw/configs/config.toml","value":"minimax","value_text":"minimax","value_type":"string"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "config_basic",
        r#"{"extra":{"action":"guard_config","candidates":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"count":2,"format":"toml","path":"/home/guagua/rustclaw/configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"valid":false}}"#,
    ));
    let payload = r#"{"applied":false,"candidates":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"count":2,"field_path":"llm.selected_vendor","message_key":"clawd.msg.config_edit.planned","path":"/home/guagua/rustclaw/configs/config.toml","reason_code":"config_edit_planned","risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"value":"minimax","would_change":false}"#;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_4", "synthesize_answer", payload));
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
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&loop_state.delivery_messages[0]).unwrap(),
        serde_json::from_str::<serde_json::Value>(payload).unwrap()
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn config_edit_renderer_uses_last_publishable_synthesis_payload() {
    let state = test_state();
    let task = claimed_task("task-config-edit-last-synthesis-payload");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    let payload = r#"{"field_path":"llm.selected_vendor","message_key":"clawd.msg.config_edit.read_guard","path":"configs/config.toml","reason_code":"config_edit_read_guard","risk_count":2,"current_value":"minimax"}"#;
    loop_state.last_publishable_synthesis_output = Some(payload.to_string());
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
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&loop_state.delivery_messages[0]).unwrap(),
        serde_json::from_str::<serde_json::Value>(payload).unwrap()
    );
    assert!(finalizer_summary.is_some());
}

#[test]
fn final_config_edit_renderer_replaces_delivery_deduped_marker() {
    let state = test_state();
    let task = claimed_task("task-config-edit-final-marker-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    let payload = r#"{"applied":false,"field_path":"llm.selected_vendor","message_key":"clawd.msg.config_edit.planned","path":"configs/config.toml","reason_code":"config_edit_planned","value":"minimax","would_change":false}"#;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_4", "synthesize_answer", payload));
    let mut finalizer_summary = None;
    let mut delivery_deduped = vec!["llm.selected_vendor".to_string()];

    let rendered = replace_config_edit_machine_marker_delivery(
        &state,
        &task,
        "preview config edit",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_deduped,
    );

    assert!(rendered);
    assert_eq!(delivery_deduped, vec![payload.to_string()]);
    assert_eq!(loop_state.delivery_messages, vec![payload.to_string()]);
    let trace = loop_state
        .output_vars
        .get("finalizer.renderer_trace.config_edit_observed_answer")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .expect("renderer trace output var");
    assert_eq!(trace["disposition"], "rendered");
    assert!(finalizer_summary.is_some());
}

#[test]
fn final_config_edit_renderer_replaces_marker_final_answer_projection() {
    let state = test_state();
    let task = claimed_task("task-config-edit-final-answer-marker-replace");
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    let payload = r#"{"applied":false,"field_path":"llm.selected_vendor","message_key":"clawd.msg.config_edit.planned","path":"configs/config.toml","reason_code":"config_edit_planned","value":"minimax","would_change":false}"#;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_4", "synthesize_answer", payload));
    let mut finalizer_summary = None;
    let mut delivery_deduped = vec![
        "llm.selected_vendor".to_string(),
        r#"{"message_key":"clawd.msg.execution.summary","step_count":4}"#.to_string(),
    ];

    let rendered = replace_config_edit_machine_marker_final_answer(
        &state,
        &task,
        "preview config edit",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_deduped,
    );

    assert!(rendered);
    assert_eq!(delivery_deduped, vec![payload.to_string()]);
    assert_eq!(loop_state.delivery_messages, vec![payload.to_string()]);
    assert!(finalizer_summary.is_some());
}

#[test]
fn scalar_replacement_prefers_config_payload_over_field_path_marker() {
    let state = test_state();
    let task = claimed_task("task-config-edit-scalar-marker-replace");
    let route = scalar_route_result();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..crate::agent_engine::AgentRunContext::default()
    };
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.delivery_messages = vec!["llm.selected_vendor".to_string()];
    loop_state.last_user_visible_respond = Some("llm.selected_vendor".to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "config_basic",
        r#"{"extra":{"action":"extract_field","exists":true,"field_path":"llm.selected_vendor","format":"toml","match_count":1,"match_strategy":"exact_path","path":"configs/config.toml","resolved_field_path":"llm.selected_vendor","resolved_path":"/home/guagua/rustclaw/configs/config.toml","value":"minimax","value_text":"minimax","value_type":"string"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "config_basic",
        r#"{"extra":{"action":"guard_config","candidates":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"count":2,"format":"toml","path":"configs/config.toml","resolved_path":"/home/guagua/rustclaw/configs/config.toml","risk_count":2,"risks":["tools.allow_sudo=true","tools.allow_path_outside_workspace=true"],"valid":false}}"#,
    ));
    let mut finalizer_summary = None;

    let rendered = replace_delivery_with_direct_scalar_observed_answer(
        &state,
        &task,
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
    );

    assert!(rendered);
    let payload: serde_json::Value =
        serde_json::from_str(&loop_state.delivery_messages[0]).expect("structured payload");
    assert_eq!(
        payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str),
        Some("config_edit_read_guard")
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
        Some(2)
    );
    assert!(finalizer_summary.is_some());
}

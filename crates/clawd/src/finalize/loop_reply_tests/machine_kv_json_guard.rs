use super::*;

#[test]
fn requested_machine_kv_summary_preserves_exact_required_field_json() {
    let task = claimed_task("task-machine-kv-required-json");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    let current = r#"{"created_files":["calc_core.py","test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut finalizer_summary = None;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::turn_context::TurnAnalysis {
            turn_type: Some(crate::turn_context::TurnType::TaskRequest),
            target_task_policy: Some(crate::turn_context::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_machine_fields": ["created_files", "test_command", "test_status"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "Return created_files, test_command, and test_status.",
        &mut loop_state,
        Some(&agent_run_context),
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec![current]);
}

#[test]
fn requested_machine_kv_summary_preserves_requested_token_json_without_state_patch() {
    let task = claimed_task("task-machine-kv-request-token-json");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    let current = r#"{"created_files":["calc_core.py","test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    let mut delivery_messages = vec![current.to_string()];
    loop_state.last_user_visible_respond = Some(current.to_string());
    let mut finalizer_summary = None;

    assert!(!replace_delivery_with_requested_machine_kv_summary(
        &task,
        "最终仅输出 JSON，包含 created_files、test_command、test_status。",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec![current]);
}

#[test]
fn requested_machine_kv_summary_restores_latest_requested_token_json() {
    let task = claimed_task("task-machine-kv-restore-latest-json");
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    let json_answer = r#"{"changed_files":["calc_core.py","test_calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed","functions":["add","sub","mul"]}"#;
    loop_state.last_publishable_synthesis_output = Some(json_answer.to_string());
    let mut delivery_messages = vec![
        "changed_files=[\"calc_core.py\",\"test_calc_core.py\"] test_command test_status"
            .to_string(),
    ];
    let mut finalizer_summary = None;

    assert!(replace_delivery_with_requested_machine_kv_summary(
        &task,
        "最后只输出 JSON，包含 changed_files、test_command、test_status、functions。",
        &mut loop_state,
        None,
        &mut finalizer_summary,
        &mut delivery_messages,
    ));
    assert_eq!(delivery_messages, vec![json_answer]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(json_answer)
    );
}

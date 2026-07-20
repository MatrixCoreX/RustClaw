use super::*;

#[test]
fn content_evidence_contractual_terminal_answer_is_kept_before_meta_classifier() {
    let answer = "最先该做的是：验证配置能否正确加载。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(answer.to_string());
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", answer));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "release_checklist.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(content_evidence_terminal_respond_is_contractual_answer(
        &loop_state,
        Some(&agent_run_context),
        answer,
    ));
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            answer,
        ),
        Some(false)
    );
}

#[test]
fn content_evidence_one_sentence_terminal_answer_is_kept_without_domain_routing() {
    let answer = "最先该做的是**验证配置能正确加载**。";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(answer.to_string());
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"release_checklist.md"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", answer));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(content_evidence_terminal_respond_is_contractual_answer(
        &loop_state,
        Some(&agent_run_context),
        answer,
    ));
}

#[test]
fn content_evidence_keeps_strict_json_projection_before_meta_classifier() {
    let answer = r#"{"created_files":["/workspace/calc_core.py"],"test_command":"python3 test_calc_core.py","test_status":"passed"}"#;
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(answer.to_string());
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.last_publishable_synthesis_output = Some(answer.to_string());
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_publishable".to_string(),
        "true".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_output".to_string(),
        answer.to_string(),
    );
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"status":"ok","path":"/workspace/calc_core.py"}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "run_cmd",
        "Ran 7 tests in 0.001s\nOK\n",
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_3", "synthesize_answer", answer));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Strict;
    route.requires_content_evidence = true;
    route.configure_exact_command_output();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            answer,
        ),
        Some(false)
    );
    assert!(content_evidence_terminal_respond_is_contractual_answer(
        &loop_state,
        Some(&agent_run_context),
        answer,
    ));
}

#[test]
fn content_evidence_scalar_heading_terminal_answer_is_kept_before_meta_classifier() {
    let answer = "Service Notes";
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(answer.to_string());
    loop_state.delivery_messages.push(answer.to_string());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|fixture body","path":"service_notes.md"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|# Service Notes\\n2|\\n3|fixture body\",\"path\":\"service_notes.md\"}"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step_result("step_2", "respond", answer));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "service_notes.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            answer,
        ),
        Some(false)
    );
    assert!(content_evidence_terminal_respond_is_contractual_answer(
        &loop_state,
        Some(&agent_run_context),
        answer,
    ));
}

#[test]
fn content_evidence_contractual_terminal_answer_requires_observation() {
    let answer = "配置加载检查应先做。";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(ok_step_result("step_1", "respond", answer));
    let mut route = free_route_result();
    route.response_shape = crate::OutputResponseShape::OneSentence;
    route.requires_content_evidence = true;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };

    assert!(!content_evidence_terminal_respond_is_contractual_answer(
        &loop_state,
        Some(&agent_run_context),
        answer,
    ));
}

#[test]
fn raw_listing_passthrough_is_dropped_for_content_evidence_free_shape() {
    let listing = "base_skill_response_contract.md\nskill_integration_guide.md";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some(listing.to_string());
    loop_state.delivery_messages.push(listing.to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "list_dir".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(format!("{listing}\n")),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::None,
        locator_hint: "docs".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            listing
        ),
        Some(true)
    );
}

#[test]
fn single_listing_entry_passthrough_is_dropped_for_content_evidence() {
    let listing = "base_skill_response_contract.md\nskill_integration_guide.md";
    let mut loop_state = crate::agent_engine::LoopState::new(2);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_user_visible_respond = Some("base_skill_response_contract.md".to_string());
    loop_state
        .delivery_messages
        .push("base_skill_response_contract.md".to_string());
    loop_state.executed_step_results.push(StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "list_dir".to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(format!("{listing}\n")),
        error: None,
        started_at: 0,
        finished_at: 0,
    });
    let route = crate::IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        delivery_intent: crate::OutputDeliveryIntent::None,
        locator_hint: "docs".to_string(),
        selection: crate::OutputSelectionContract::default(),
    };
    let agent_run_context = crate::agent_engine::AgentRunContext {
        output_contract: Some(route.clone()),
        auto_locator_path: Some("/tmp/docs".to_string()),
        ..Default::default()
    };
    assert_eq!(
        should_drop_passthrough_delivery_for_content_evidence(
            &loop_state,
            true,
            Some(&agent_run_context),
            "base_skill_response_contract.md"
        ),
        Some(true)
    );
}

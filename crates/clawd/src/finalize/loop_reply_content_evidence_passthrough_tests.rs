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
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "release_checklist.md".to_string();
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
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
fn content_evidence_one_sentence_terminal_answer_is_kept_without_semantic_kind() {
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
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

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
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
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
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
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
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::DirectoryPurposeSummary,
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
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

use super::{
    bare_topic_clarify_question_should_drop_context_target,
    bare_topic_memory_expansion_route_should_defer_to_agent_loop,
    bare_topic_model_supplied_locator_route_should_defer_to_agent_loop,
};

fn executable_filename_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "read README and summarize".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            locator_kind: crate::OutputLocatorKind::Filename,
            locator_hint: "README.md".to_string(),
            requires_content_evidence: true,
            ..Default::default()
        },
    }
}

fn empty_session_snapshot() -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

#[test]
fn bare_topic_command_marker_with_unmentioned_context_target_forces_clarify() {
    let mut route = executable_filename_route();
    route.resolved_intent = "View logs from the ops_http_repair test suite".to_string();
    route.route_reason =
        "command_payload_requires_raw_output_execution; context mentioned scripts/nl_suite_logs/ops_http_repair"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let snapshot = empty_session_snapshot();

    assert!(
        bare_topic_memory_expansion_route_should_defer_to_agent_loop(
            "logs", &route, None, &snapshot,
        )
    );
}

#[test]
fn bare_topic_raw_command_without_command_marker_stays_executable() {
    let mut route = executable_filename_route();
    route.resolved_intent = "View logs from the ops_http_repair test suite".to_string();
    route.route_reason =
        "model emitted raw command output semantic without a command observation marker"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let snapshot = empty_session_snapshot();

    assert!(
        !bare_topic_memory_expansion_route_should_defer_to_agent_loop(
            "logs", &route, None, &snapshot,
        )
    );
}

#[test]
fn bare_topic_model_supplied_locator_forces_clarify() {
    let mut route = executable_filename_route();
    route.resolved_intent = "View logs from /workspace/logs".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/workspace/logs".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    let snapshot = empty_session_snapshot();
    assert!(
        bare_topic_model_supplied_locator_route_should_defer_to_agent_loop(
            "logs", &route, None, &snapshot,
        )
    );
}

#[test]
fn bare_topic_clarify_question_with_unmentioned_context_target_is_sanitized() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.resolved_intent = "View logs".to_string();
    route.clarify_question =
        "Which logs: ops_http_repair, a specific file path, or current process logs?".to_string();

    assert!(bare_topic_clarify_question_should_drop_context_target(
        "logs", &route
    ));
}

#[test]
fn bare_topic_model_supplied_locator_allows_active_clarify_locator_reply() {
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Continue previous clarified request using the supplied target directory".to_string();
    route.route_reason =
        "preserve_active_clarify_output_contract; active_clarify_locator_reply_bound_for_loop"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "scripts".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    let snapshot = empty_session_snapshot();
    assert!(
        !bare_topic_model_supplied_locator_route_should_defer_to_agent_loop(
            "scripts", &route, None, &snapshot,
        )
    );
}

#[test]
fn bare_topic_model_supplied_locator_allows_reused_structured_anchor() {
    let mut route = executable_filename_route();
    route.resolved_intent = "List contents of the document subdirectory".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({"active_selection": {"target": "document"}})),
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            bound_target: Some("/workspace".to_string()),
            ordered_entries: vec!["document".to_string()],
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !bare_topic_model_supplied_locator_route_should_defer_to_agent_loop(
            "document",
            &route,
            Some(&analysis),
            &snapshot,
        )
    );
}

#[test]
fn bare_topic_model_supplied_locator_keeps_existing_clarify() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.ask_mode = crate::AskMode::clarify_trace();
    route.resolved_intent = "View logs from /workspace/logs".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/workspace/logs".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    let snapshot = empty_session_snapshot();
    assert!(
        bare_topic_model_supplied_locator_route_should_defer_to_agent_loop(
            "logs", &route, None, &snapshot,
        )
    );
}

#[test]
fn bare_topic_model_supplied_locator_preserves_non_bare_request() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Check target directory size".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/workspace/target".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;

    let snapshot = empty_session_snapshot();
    assert!(
        !bare_topic_model_supplied_locator_route_should_defer_to_agent_loop(
            "check target size",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn chinese_deictic_delivery_sentence_is_not_treated_as_bare_topic() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.resolved_intent = "把最近提到的文件发给用户".to_string();
    route.clarify_question = "请提供目标文件或目录的具体路径。".to_string();

    assert!(!bare_topic_clarify_question_should_drop_context_target(
        "把那个文件发给我",
        &route
    ));
}

use super::{
    execution_user_request, sanitize_untrusted_normalizer_answer_candidate_for_execution,
    sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution,
    should_preserve_original_inline_structured_input,
};

fn empty_session_snapshot() -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

fn base_route(ask_mode: crate::AskMode, resolved_intent: &str) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode,
        resolved_intent: resolved_intent.to_string(),
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
        output_contract: crate::IntentOutputContract::default(),
    }
}

fn route_reason_has_marker(route_result: &crate::RouteResult, marker: &str) -> bool {
    super::super::route_reason_has_marker(route_result, marker)
}

#[test]
fn untrusted_multiline_answer_candidate_is_removed_from_execution_context() {
    let mut route = base_route(
        crate::AskMode::direct_answer(),
        "Draft release note\nanswer_candidate: Login module update\nOAuth details\n60% faster",
    );
    let mut resolved =
        "Draft release note\nanswer_candidate: Login module update\nOAuth details\n### RUNTIME_CONTEXT\nworkspace_root: /tmp/repo"
            .to_string();
    let mut prompt_with_memory =
        "### MEMORY_CONTEXT\n<none>\nDraft release note\nanswer_candidate: Login module update\nOAuth details\n### RUNTIME_CONTEXT\nworkspace_root: /tmp/repo"
            .to_string();

    sanitize_untrusted_normalizer_answer_candidate_for_execution(
        &mut route,
        "Draft release note",
        "<none>",
        &empty_session_snapshot(),
        &mut resolved,
        &mut prompt_with_memory,
    );

    assert!(!route.resolved_intent.contains("answer_candidate:"));
    assert!(!route.resolved_intent.contains("OAuth details"));
    assert!(!resolved.contains("answer_candidate:"));
    assert!(!resolved.contains("OAuth details"));
    assert!(resolved.contains("### RUNTIME_CONTEXT"));
    assert!(!prompt_with_memory.contains("answer_candidate:"));
    assert!(route_reason_has_marker(
        &route,
        "untrusted_normalizer_answer_candidate_removed_from_execution_context"
    ));
}

#[test]
fn compact_machine_literal_answer_candidate_stays_in_execution_context() {
    let mut route = base_route(
        crate::AskMode::direct_answer(),
        "Draft runtime note\nanswer_candidate: Use Python 3.11 for this runtime.",
    );
    let mut resolved =
        "Draft runtime note\nanswer_candidate: Use Python 3.11 for this runtime.".to_string();
    let mut prompt_with_memory = resolved.clone();

    sanitize_untrusted_normalizer_answer_candidate_for_execution(
        &mut route,
        "Write a runtime note mentioning Python 3.11",
        "<none>",
        &empty_session_snapshot(),
        &mut resolved,
        &mut prompt_with_memory,
    );

    assert!(route.resolved_intent.contains("answer_candidate:"));
    assert!(resolved.contains("Python 3.11"));
    assert!(!route_reason_has_marker(
        &route,
        "untrusted_normalizer_answer_candidate_removed_from_execution_context"
    ));
}

#[test]
fn pure_direct_chat_freeform_rewrite_uses_current_user_request() {
    let mut route = base_route(
        crate::AskMode::direct_answer(),
        "Draft a release note for project RustClaw.",
    );
    let mut resolved = route.resolved_intent.clone();
    let mut prompt_with_memory = format!("### MEMORY_CONTEXT\n<none>\n{}", resolved);

    sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution(
        &mut route,
        "Draft a release note.",
        None,
        &mut resolved,
        &mut prompt_with_memory,
    );

    assert_eq!(route.resolved_intent, "Draft a release note.");
    assert_eq!(resolved, "Draft a release note.");
    assert_eq!(prompt_with_memory, "Draft a release note.");
    assert!(route_reason_has_marker(
        &route,
        "untrusted_normalizer_freeform_rewrite_removed_from_execution_context"
    ));
}

#[test]
fn standalone_task_request_freeform_rewrite_uses_current_user_request() {
    let mut route = base_route(
        crate::AskMode::direct_answer(),
        "Create a project-specific test plan after inspecting workspace evidence.",
    );
    let mut resolved = route.resolved_intent.clone();
    let mut prompt_with_memory = resolved.clone();
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution(
        &mut route,
        "Create a test plan.",
        Some(&turn_analysis),
        &mut resolved,
        &mut prompt_with_memory,
    );

    assert_eq!(route.resolved_intent, "Create a test plan.");
    assert_eq!(resolved, "Create a test plan.");
    assert!(route_reason_has_marker(
        &route,
        "untrusted_normalizer_freeform_rewrite_removed_from_execution_context"
    ));
}

#[test]
fn standalone_task_request_freeform_rewrite_preserves_active_anchor_context() {
    let mut route = base_route(
        crate::AskMode::direct_answer(),
        "Read the second active ordered entry.",
    );
    let anchor = "\n\n### ACTIVE_EXECUTION_ANCHOR\nfollowup_bound_target: /tmp/logs\nfollowup_ordered_entries: 1:a.log | 2:b.log";
    let mut resolved = format!("{}{}", route.resolved_intent, anchor);
    let mut prompt_with_memory = format!(
        "{}\n\n### RUNTIME_CONTEXT\nworkspace_root: /tmp{}",
        route.resolved_intent, anchor
    );
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution(
        &mut route,
        "show the second one",
        Some(&turn_analysis),
        &mut resolved,
        &mut prompt_with_memory,
    );

    assert_eq!(route.resolved_intent, "show the second one");
    assert!(resolved.starts_with("show the second one\n\n### ACTIVE_EXECUTION_ANCHOR"));
    assert!(resolved.contains("followup_ordered_entries: 1:a.log | 2:b.log"));
    assert!(prompt_with_memory.starts_with("show the second one\n\n### RUNTIME_CONTEXT"));
    assert!(prompt_with_memory.contains("followup_bound_target: /tmp/logs"));
    assert!(route_reason_has_marker(
        &route,
        "untrusted_normalizer_freeform_rewrite_removed_from_execution_context"
    ));
}

#[test]
fn active_task_correction_keeps_resolved_context_rewrite() {
    let mut route = base_route(
        crate::AskMode::direct_answer(),
        "Current task:\nold draft\nNew user instruction:\nmake it shorter",
    );
    let original_resolved = route.resolved_intent.clone();
    let mut resolved = original_resolved.clone();
    let mut prompt_with_memory = resolved.clone();
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({"format": "shorter"})),
        attachment_processing_required: false,
    };

    sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution(
        &mut route,
        "make it shorter",
        Some(&turn_analysis),
        &mut resolved,
        &mut prompt_with_memory,
    );

    assert_eq!(route.resolved_intent, original_resolved);
    assert_eq!(resolved, original_resolved);
    assert!(!route_reason_has_marker(
        &route,
        "untrusted_normalizer_freeform_rewrite_removed_from_execution_context"
    ));
}

#[test]
fn evidence_route_keeps_resolved_prompt_rewrite() {
    let mut route = base_route(
        crate::AskMode::planner_execute_plain(),
        "Summarize the current workspace using evidence.",
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut resolved = route.resolved_intent.clone();
    let mut prompt_with_memory = resolved.clone();

    sanitize_untrusted_normalizer_freeform_rewrite_for_direct_chat_execution(
        &mut route,
        "Summarize it.",
        None,
        &mut resolved,
        &mut prompt_with_memory,
    );

    assert_eq!(
        route.resolved_intent,
        "Summarize the current workspace using evidence."
    );
    assert_eq!(resolved, "Summarize the current workspace using evidence.");
    assert!(!route_reason_has_marker(
        &route,
        "untrusted_normalizer_freeform_rewrite_removed_from_execution_context"
    ));
}

#[test]
fn inline_json_payload_prefers_original_user_request_for_execution() {
    let prompt = r#"Sort this JSON array by score descending: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let resolved =
        "Sort the provided JSON array by score in descending order and output as a table";
    assert!(should_preserve_original_inline_structured_input(
        prompt, resolved
    ));
    assert_eq!(execution_user_request(prompt, resolved), prompt);
}

#[test]
fn non_structured_prompt_keeps_resolved_execution_request() {
    let prompt = "Check whether telegramd is currently running and briefly explain the status";
    let resolved = "Check the current telegramd process status and summarize it briefly";
    assert!(!should_preserve_original_inline_structured_input(
        prompt, resolved
    ));
    assert_eq!(execution_user_request(prompt, resolved), resolved);
}

use super::{
    deictic_bare_locator_should_defer_to_agent_loop,
    deictic_memory_only_route_should_defer_to_agent_loop, deictic_missing_locator_reason_code,
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

fn turn_analysis_with_state_patch(
    state_patch: serde_json::Value,
) -> crate::intent_router::TurnAnalysis {
    crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(state_patch),
        attachment_processing_required: false,
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

fn unresolved_deictic_analysis() -> crate::intent_router::TurnAnalysis {
    crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "deictic_reference": {"target": "unresolved_prior_object"}
        })),
        attachment_processing_required: false,
    }
}

fn snapshot_with_logs_ordered_entries() -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "list logs files".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("logs".to_string()),
            ordered_entries: vec!["act_plan.log".to_string(), "clawd-dev.log".to_string()],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

#[test]
fn deictic_bare_locator_forces_clarify_before_auto_locator() {
    let route = executable_filename_route();
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
    );
    assert!(deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        Some(&analysis),
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_directory_scope_with_target_filename_forces_clarify() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "case_only".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
    );
    assert!(deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        Some(&analysis),
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_synthesized_relative_path_forces_clarify() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "reports/report.md".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
    );
    assert!(deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        Some(&analysis),
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_defer_uses_machine_reason_code() {
    let mut route = executable_filename_route();
    route.route_reason = "search_locator_required".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "reports/report.md".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    assert_eq!(
        deictic_missing_locator_reason_code(&route),
        "missing_search_locator"
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPathSummary;
    assert_eq!(
        deictic_missing_locator_reason_code(&route),
        "missing_search_locator"
    );
}

#[test]
fn deictic_file_locator_with_filename_hint_still_allows_auto_locator() {
    let route = executable_filename_route();
    assert!(!deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        None,
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_explicit_path_still_allows_auto_locator() {
    let route = executable_filename_route();
    assert!(!deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        None,
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_context_bound_path_still_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
            .to_string();
    assert!(!deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        None,
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_result_reference_with_two_named_files_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"comparison_result"}}),
    );
    assert!(!deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        Some(&analysis),
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_result_reference_after_command_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"current_action_result"}}),
    );

    assert!(!deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        Some(&analysis),
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_target_before_followup_still_forces_clarify() {
    let route = executable_filename_route();
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
    );
    assert!(deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        Some(&analysis),
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_directory_reference_after_named_folder_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "docs".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"current_turn_locator"}}),
    );

    assert!(!deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        Some(&analysis),
        &empty_session_snapshot(),
    ));
}

#[test]
fn direct_bare_locator_still_allows_auto_locator() {
    let route = executable_filename_route();
    assert!(!deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        None,
        &empty_session_snapshot(),
    ));
}

#[test]
fn deictic_memory_only_execute_route_requires_clarify_without_session_anchor() {
    let mut route = executable_filename_route();
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    let analysis = unresolved_deictic_analysis();
    assert!(deictic_memory_only_route_should_defer_to_agent_loop(
        "看看那个目录下面都有什么",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn deictic_guards_allow_locator_bound_to_active_ordered_entry() {
    let mut route = executable_filename_route();
    route.wants_file_delivery = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd-dev.log".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.requires_content_evidence = true;
    let analysis = unresolved_deictic_analysis();
    let snapshot = snapshot_with_logs_ordered_entries();

    assert!(!deictic_memory_only_route_should_defer_to_agent_loop(
        "send selected entry",
        &route,
        Some(&analysis),
        &snapshot,
    ));
    assert!(!deictic_bare_locator_should_defer_to_agent_loop(
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn deictic_memory_only_command_output_reference_does_not_defer_to_agent_loop() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!deictic_memory_only_route_should_defer_to_agent_loop(
        "执行 pwd，然后用一句话解释这个路径代表什么",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn deictic_memory_only_guard_allows_current_session_alias_binding() {
    let mut route = executable_filename_route();
    route.output_contract.locator_hint = "/tmp/docs".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "那个目录".to_string(),
                target: "/tmp/docs".to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!deictic_memory_only_route_should_defer_to_agent_loop(
        "看看那个目录下面都有什么",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn deictic_memory_only_guard_rejects_session_alias_target_resolved_by_normalizer_only() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Read the first 10 lines of /tmp/device/README.md".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "that_file".to_string(),
                target: "/tmp/device/README.md".to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(deictic_memory_only_route_should_defer_to_agent_loop(
        "把那个文件开头读 10 行",
        &route,
        Some(&turn_analysis_with_state_patch(serde_json::json!({
            "deictic_reference": {"target": "missing_locator"}
        }))),
        &snapshot,
    ));
}

#[test]
fn deictic_memory_only_guard_rejects_stale_observed_target_without_route_match() {
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteSchemaVersion;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.resolved_intent =
        "Query the current workspace SQLite database schema version".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some(
                "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/archive"
                    .to_string(),
            ),
            ..Default::default()
        }),
    };

    let analysis = unresolved_deictic_analysis();
    assert!(deictic_memory_only_route_should_defer_to_agent_loop(
        "看一下那个 sqlite 的 schema version 是多少",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn deictic_memory_only_guard_allows_active_clarify_anchor() {
    let route = executable_filename_route();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Which file?".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: true,
            output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
            semantic_kind: None,
            source_request: "Send the file".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };

    assert!(!deictic_memory_only_route_should_defer_to_agent_loop(
        "把那个文件发给我",
        &route,
        None,
        &snapshot,
    ));
}

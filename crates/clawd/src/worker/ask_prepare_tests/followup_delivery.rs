use super::*;
use crate::worker::ask_prepare::repair_structural_file_delivery_resolution_for_turn;

#[test]
fn file_delivery_with_structured_locator_is_preserved() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "send the routed file".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: false,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "/tmp/model_io.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read the last 10 lines".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/model_io.log".to_string()),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(route.output_contract.locator_hint, "/tmp/model_io.log");
    assert!(route.clarify_question.is_empty());
}

#[test]
fn unresolved_file_delivery_without_locator_defers_to_loop() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "send the file".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.clarify_question.is_empty());
    assert!(route
        .route_reason
        .contains("unresolved_file_delivery_requires_locator"));
}

#[test]
fn generated_file_delivery_without_locator_can_choose_runtime_target() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "create a shell script, save it, and deliver the generated file"
            .to_string(),
        needs_clarify: true,
        route_reason: "generated_file_delivery".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: "please provide a filename".to_string(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::GeneratedFileDelivery,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::GeneratedFileDelivery
    );
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
    assert!(route.clarify_question.is_empty());
    assert!(route
        .route_reason
        .contains("generated_file_delivery_allows_runtime_target"));
}

#[test]
fn generated_file_delivery_with_filename_locator_stays_existing_file_delivery() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "deliver a filename-scoped file target".to_string(),
        needs_clarify: false,
        route_reason:
            "semantic_contract_requires_evidence; generated_file_delivery; generated_file_delivery_allows_runtime_target"
                .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::High,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::Filename,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::GeneratedFileDelivery,
            locator_hint: "definitely_missing_named_file_route_cleanup_001.txt".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Filename
    );
    assert_eq!(
        route.output_contract.locator_hint,
        "definitely_missing_named_file_route_cleanup_001.txt"
    );
    assert!(route
        .route_reason
        .contains("filename_locator_preserved_as_existing_file_delivery"));
    assert!(!route
        .route_reason
        .contains("generated_file_delivery_allows_runtime_target"));
}

#[test]
fn generated_file_delivery_current_workspace_without_locator_can_choose_runtime_target() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "deliver unresolved workspace file target".to_string(),
        needs_clarify: false,
        route_reason: "semantic_contract_requires_evidence; generated_file_delivery".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::GeneratedFileDelivery,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::GeneratedFileDelivery
    );
    assert!(route
        .route_reason
        .contains("generated_file_delivery_allows_runtime_target"));
    assert!(!route
        .route_reason
        .contains("unresolved_file_delivery_requires_clarify"));
}

#[test]
fn generated_file_delivery_path_kind_without_locator_defers_to_loop() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "deliver unresolved existing file target".to_string(),
        needs_clarify: false,
        route_reason: "semantic_contract_requires_evidence".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::GeneratedFileDelivery,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
    assert!(route
        .route_reason
        .contains("unresolved_file_delivery_requires_locator"));
    assert!(!route
        .route_reason
        .contains("generated_file_delivery_allows_runtime_target"));
}

#[test]
fn generated_file_delivery_existing_directory_locator_defers_to_loop() {
    let root = make_temp_root("generated_delivery_dir_locator");
    std::fs::write(root.join("service_notes.md"), "service notes").expect("fixture");
    std::fs::write(root.join("release_checklist.md"), "release").expect("fixture");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "deliver one selected file from a directory".to_string(),
        needs_clarify: false,
        route_reason: "semantic_contract_requires_evidence".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::GeneratedFileDelivery,
            locator_hint: root.display().to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
    assert_eq!(route.output_contract.locator_hint, "");
    assert!(route
        .route_reason
        .contains("directory_file_delivery_requires_structured_selection"));
    assert!(route
        .route_reason
        .contains("unresolved_file_delivery_requires_locator"));
    assert!(!route
        .route_reason
        .contains("generated_file_delivery_allows_runtime_target"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn structurally_resolved_file_delivery_defers_recent_read_target_to_loop() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "deliver the active file target".to_string(),
        needs_clarify: false,
        route_reason: "normalizer resolved delivery from immediate context".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read README.md head".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/README.md".to_string()),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route.resolved_intent.contains("/tmp/README.md"));
    assert!(route
        .route_reason
        .contains("unresolved_file_delivery_requires_locator"));
    assert!(!route
        .route_reason
        .contains("structural_file_delivery_bound_to_recent_read_target"));
}

#[test]
fn clarified_structural_file_delivery_defers_recent_read_target_to_loop() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "deliver active bound target".to_string(),
        needs_clarify: true,
        route_reason: "active_anchor_file_delivery_requires_structured_reference".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let target =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/configs/app_config.toml";
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read structured field".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some(target.to_string()),
            source_task_id: "task-config-field".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route.resolved_intent.contains(target));
    assert!(route
        .route_reason
        .contains("unresolved_file_delivery_requires_locator"));
    assert!(!route
        .route_reason
        .contains("structural_file_delivery_bound_to_recent_read_target"));
}

#[test]
fn ambiguous_deictic_file_delivery_does_not_bind_stale_read_target() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "deliver unresolved selected file".to_string(),
        needs_clarify: true,
        route_reason: "normalizer marked file delivery target as ambiguous".to_string(),
        route_confidence: Some(0.82),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let stale_target = "/tmp/release_checklist.md";
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read previous file".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some(stale_target.to_string()),
            source_task_id: "task-read-previous".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "deictic_reference": {"target": "ambiguous_locator"}
        })),
        attachment_processing_required: false,
    };

    repair_structural_file_delivery_resolution_for_turn(
        &mut route,
        &snapshot,
        Some(&turn_analysis),
    );

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(route.output_contract.locator_hint, "");
    assert!(!route.resolved_intent.contains(stale_target));
    assert!(!route
        .route_reason
        .contains("structural_file_delivery_bound_to_recent_read_target"));
    assert!(route
        .route_reason
        .contains("unresolved_file_delivery_requires_locator"));
}

#[test]
fn directory_selection_clarify_marker_blocks_stale_read_target_rebind() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "deliver one selected file from a directory".to_string(),
        needs_clarify: true,
        route_reason: concat!(
            "clarify_reason_code:missing_delivery_locator; ",
            "directory_file_delivery_requires_structured_selection"
        )
        .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read README.md head".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/README.md".to_string()),
            source_task_id: "task-readme".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(route.output_contract.locator_hint, "");
    assert!(!route.resolved_intent.contains("/tmp/README.md"));
    assert!(!route
        .route_reason
        .contains("structural_file_delivery_bound_to_recent_read_target"));
}

#[test]
fn structurally_resolved_file_delivery_defers_active_delivery_target_to_loop() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "deliver the active file target again".to_string(),
        needs_clarify: false,
        route_reason: "normalizer resolved delivery from immediate context".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "send release checklist".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some(
                "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
                    .to_string(),
            ),
            source_task_id: "task-delivery".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route.resolved_intent.contains("release_checklist.md"));
    assert!(route
        .route_reason
        .contains("unresolved_file_delivery_requires_locator"));
    assert!(!route
        .route_reason
        .contains("structural_file_delivery_bound_to_recent_read_target"));
}

#[test]
fn structurally_resolved_file_delivery_defers_active_observed_target_to_loop() {
    let target =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD";
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "deliver the active file target".to_string(),
        needs_clarify: false,
        route_reason: "normalizer resolved delivery from immediate context".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some(target.to_string()),
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            ..Default::default()
        }),
    };

    repair_structural_file_delivery_resolution(&mut route, &snapshot);

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route.resolved_intent.contains(target));
    assert!(route
        .route_reason
        .contains("unresolved_file_delivery_requires_locator"));
    assert!(!route
        .route_reason
        .contains("structural_file_delivery_bound_to_recent_read_target"));
}

#[test]
fn ordered_entry_reference_binds_third_delivery_from_active_frame() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "deliver the third listed file".to_string(),
        needs_clarify: false,
        route_reason: "normalizer selected an ordinal follow-up".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "/home/guagua/rustclaw/logs/clawd.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("logs".to_string()),
            ordered_entries: vec![
                "act_plan.log".to_string(),
                "clawd.log".to_string(),
                "clawd.run.log".to_string(),
                "clawd.test.log".to_string(),
            ],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(json!({"ordered_entry_ref":{"index":3,"index_base":1}})),
        attachment_processing_required: false,
    };

    assert!(bind_ordered_entry_reference_from_active_frame(
        &mut route,
        &snapshot,
        Some(&analysis),
        None
    ));

    assert_eq!(route.output_contract.locator_hint, "logs/clawd.run.log");
    assert!(route
        .route_reason
        .contains("ordered_entry_reference_bound_from_active_frame"));
    assert!(route.resolved_intent.contains("logs/clawd.run.log"));
}

#[test]
fn ordered_entry_reference_repairs_conflicting_index_from_route_path_token() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "deliver selected path /home/guagua/rustclaw/logs/clawd.codex.nltest.log"
            .to_string(),
        needs_clarify: false,
        route_reason: "selected file path is /home/guagua/rustclaw/logs/clawd.codex.nltest.log"
            .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "先列出 logs 目录下前 4 个文件名".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/home/guagua/rustclaw/logs".to_string()),
            ordered_entries: vec![
                "act_plan.log".to_string(),
                "clawd-dev.log".to_string(),
                "clawd.codex.nltest.log".to_string(),
                "clawd.log".to_string(),
            ],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(json!({"ordered_entry_ref":{"index":4,"index_base":1}})),
        attachment_processing_required: false,
    };

    assert!(bind_ordered_entry_reference_from_active_frame(
        &mut route,
        &snapshot,
        Some(&analysis),
        None
    ));

    assert_eq!(
        route.output_contract.locator_hint,
        "/home/guagua/rustclaw/logs/clawd.codex.nltest.log"
    );
    assert!(route
        .route_reason
        .contains("ordered_entry_reference_index_repaired_from_route_path"));
}

#[test]
fn ordered_entry_reference_binds_previous_from_selected_entry() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "read previous selected file tail".to_string(),
        needs_clarify: false,
        route_reason: "normalizer selected a relative ordinal follow-up".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "/home/guagua/rustclaw/logs/clawd.run.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "把第三个发给我".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some("logs/clawd.run.log".to_string()),
            ordered_entries: vec![
                "act_plan.log".to_string(),
                "clawd.log".to_string(),
                "clawd.run.log".to_string(),
                "clawd.test.log".to_string(),
            ],
            selected_entry_index: Some(2),
            source_task_id: "task-delivery".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(json!({"ordered_entry_ref":{"relative_offset":-1}})),
        attachment_processing_required: false,
    };

    assert!(bind_ordered_entry_reference_from_active_frame(
        &mut route,
        &snapshot,
        Some(&analysis),
        None
    ));

    assert_eq!(route.output_contract.locator_hint, "logs/clawd.log");
    assert!(route.resolved_intent.contains("logs/clawd.log"));
}

#[test]
fn ordered_entry_reference_binds_scalar_path_from_active_frame() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "return only the selected path".to_string(),
        needs_clarify: false,
        route_reason: "normalizer selected an active ordered entry; scalar_path_only".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "list matching files".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/rustclaw/fuzzy_top3".to_string()),
            ordered_entries: vec![
                "abcd_report.md".to_string(),
                "my_abcd.txt".to_string(),
                "x_abcd_log.txt".to_string(),
                "zz_abcd_backup.log".to_string(),
            ],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(json!({"ordered_entry_ref":{"index":4,"index_base":1}})),
        attachment_processing_required: false,
    };

    assert!(bind_ordered_entry_reference_from_active_frame(
        &mut route,
        &snapshot,
        Some(&analysis),
        None
    ));

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        "/tmp/rustclaw/fuzzy_top3/zz_abcd_backup.log"
    );
    assert!(route
        .route_reason
        .contains("ordered_entry_reference_bound_from_active_frame"));
}

#[test]
fn content_read_followup_reuses_active_delivery_target_without_prompt_locator() {
    let target = "/home/guagua/rustclaw/logs/clawd.codex.nltest.log";
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent:
            "Read the last line of the file clawd-dev.log (slice_mode=tail, slice_n=1)".to_string(),
        needs_clarify: false,
        route_reason:
            "normalizer chose previous file clawd-dev.log; semantic_contract_requires_evidence"
                .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Filename,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            locator_hint: "clawd-dev.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "deliver selected log".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some(target.to_string()),
            source_task_id: "task-delivery".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(bind_content_read_to_active_delivery_target(
        &mut route,
        &snapshot,
        None,
        "read tail 1"
    ));

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(route.output_contract.locator_hint, target);
    assert!(route.resolved_intent.contains(target));
    assert!(route.resolved_intent.contains("slice_mode=tail"));
    assert!(route.resolved_intent.contains("slice_n=1"));
    assert!(!route.resolved_intent.contains("clawd-dev.log"));
    assert!(route
        .route_reason
        .contains("active_delivery_content_target_bound"));
}

#[test]
fn content_read_followup_keeps_explicit_current_prompt_locator() {
    let target = "/home/guagua/rustclaw/logs/clawd.codex.nltest.log";
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "Read clawd-dev.log tail 1".to_string(),
        needs_clarify: false,
        route_reason: "current prompt supplied concrete filename".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Filename,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            locator_hint: "clawd-dev.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "deliver selected log".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some(target.to_string()),
            source_task_id: "task-delivery".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!bind_content_read_to_active_delivery_target(
        &mut route,
        &snapshot,
        None,
        "read clawd-dev.log tail 1"
    ));

    assert_eq!(route.output_contract.locator_hint, "clawd-dev.log");
    assert!(!route
        .route_reason
        .contains("active_delivery_content_target_bound"));
}

#[test]
fn content_read_followup_repaired_active_task_binding_overrides_ordered_entry() {
    let target = "/home/guagua/rustclaw/logs/clawd.codex.nltest.log";
    let stale = "/home/guagua/rustclaw/logs/clawd-dev.log";
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: format!(
            "Read ordered entry index 2 last line\nordered_entry_target: {stale}"
        ),
        needs_clarify: false,
        route_reason:
            "llm_semantic_contract_repair:active_task_invalid_turn_binding_repaired_to_canonical_active_task_continuation_for_file_slice_correction; ordered_entry_reference_bound_from_active_frame; semantic_contract_requires_evidence"
                .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            locator_hint: stale.to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "deliver selected log".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some(target.to_string()),
            source_task_id: "task-delivery".to_string(),
            ordered_entries: vec![
                "act_plan.log".to_string(),
                "clawd-dev.log".to_string(),
                "clawd.codex.nltest.log".to_string(),
            ],
            selected_entry_index: Some(2),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(json!({"ordered_entry_ref":{"index":2,"index_base":1}})),
        attachment_processing_required: false,
    };

    assert!(bind_content_read_to_active_delivery_target(
        &mut route,
        &snapshot,
        Some(&analysis),
        "read tail 1"
    ));

    assert_eq!(route.output_contract.locator_hint, target);
    assert!(route.resolved_intent.contains(target));
    assert!(!route.resolved_intent.contains(stale));
    assert!(route
        .route_reason
        .contains("active_delivery_content_target_bound"));
}

#[test]
fn content_read_followup_rewrites_only_structural_route_reason_locator() {
    let target = "/home/guagua/rustclaw/logs/clawd.codex.nltest.log";
    let stale = "/home/guagua/rustclaw/logs/clawd-dev.log";
    let stale_with_suffix = format!("{stale}.backup");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: format!("Read selected log\nordered_entry_target: {stale}"),
        needs_clarify: false,
        route_reason: format!(
            "locator_hint: {stale}; unrelated_path_token: {stale_with_suffix}; semantic_contract_requires_evidence"
        ),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            locator_hint: stale.to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "deliver selected log".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some(target.to_string()),
            source_task_id: "task-delivery".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(bind_content_read_to_active_delivery_target(
        &mut route,
        &snapshot,
        None,
        "read tail 1"
    ));

    assert!(route
        .route_reason
        .contains(&format!("locator_hint: {target}")));
    assert!(route.route_reason.contains(&stale_with_suffix));
    assert!(route
        .route_reason
        .contains("active_delivery_content_target_bound"));
}

#[test]
fn content_read_followup_rewrites_only_structural_resolved_intent_locator() {
    let target = "/home/guagua/rustclaw/logs/clawd.codex.nltest.log";
    let stale = "/home/guagua/rustclaw/logs/clawd-dev.log";
    let stale_with_suffix = format!("{stale}.backup");
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: format!(
            "Read selected log\nordered_entry_target: {stale}\nunrelated_path_token: {stale_with_suffix}"
        ),
        needs_clarify: false,
        route_reason: "semantic_contract_requires_evidence".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            locator_hint: stale.to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "deliver selected log".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some(target.to_string()),
            source_task_id: "task-delivery".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(bind_content_read_to_active_delivery_target(
        &mut route,
        &snapshot,
        None,
        "read tail 1"
    ));

    assert!(route
        .resolved_intent
        .lines()
        .any(|line| line.trim() == format!("ordered_entry_target: {target}")));
    assert!(route.resolved_intent.contains(&stale_with_suffix));
    assert!(!route
        .resolved_intent
        .lines()
        .any(|line| line.trim() == format!("ordered_entry_target: {stale}")));
    assert!(route
        .route_reason
        .contains("active_delivery_content_target_bound"));
}

#[test]
fn active_delivery_content_target_token_survives_task_turn_merge_prompt() {
    let target = "/home/guagua/rustclaw/logs/clawd.codex.nltest.log";
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        route_reason: "semantic_contract_requires_evidence; active_delivery_content_target_bound"
            .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            locator_hint: target.to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let mut runtime_prompt =
        "Current task:\nlist logs\n\nNew user instruction:\nread tail 1".to_string();

    append_active_delivery_content_target_token(&mut runtime_prompt, &route);

    assert!(runtime_prompt.contains("active_delivery_content_target:"));
    assert!(runtime_prompt.contains(target));

    append_active_delivery_content_target_token(&mut runtime_prompt, &route);
    assert_eq!(runtime_prompt.matches(target).count(), 1);

    let mut suffix_prompt = format!("Current task:\narchived target: {target}.backup");
    append_active_delivery_content_target_token(&mut suffix_prompt, &route);
    assert!(suffix_prompt
        .lines()
        .any(|line| line.trim() == format!("active_delivery_content_target: {target}")));
}

#[test]
fn ordered_entry_reference_infers_exact_current_prompt_token_from_active_frame() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "List or describe documentation files in the current directory"
            .to_string(),
        needs_clarify: false,
        route_reason: "The request 'document' is ambiguous without a concrete locator".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "/home/guagua/rustclaw".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "看看那个目录下面都有什么".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/home/guagua/rustclaw".to_string()),
            ordered_entries: vec![
                "configs".to_string(),
                "crates".to_string(),
                "docs".to_string(),
                "document".to_string(),
            ],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(bind_ordered_entry_reference_from_active_frame(
        &mut route,
        &snapshot,
        None,
        Some("document")
    ));

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        "/home/guagua/rustclaw/document"
    );
    assert!(route
        .route_reason
        .contains("ordered_entry_reference_inferred_from_current_prompt_token"));
    assert!(route
        .route_reason
        .contains("ordered_entry_reference_bound_from_active_frame"));
}

#[test]
fn filename_only_output_patch_clears_file_delivery_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "only output the basename of the previously delivered file".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::FileToken,
            requires_content_evidence: false,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "/tmp/README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(json!({"output_format": "filename_only"})),
        attachment_processing_required: false,
    };

    super::super::clear_file_delivery_contract_for_filename_only(&mut route, Some(&analysis));

    assert!(!route.wants_file_delivery);
    assert!(!route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::None
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert!(route
        .route_reason
        .contains("filename_only_output_clears_file_delivery_contract"));
}

#[test]
fn immediate_last_turn_clarify_placeholder_is_detected() {
    assert!(crate::intent::continuation_resolver::immediate_prior_turn_was_clarify(
        "### LAST_TURN_FULL\n[TURN -1]\nUser: 读取待确认文件里的名字字段，只输出值\nAssistant: [clarification_requested]\n[/TURN]"
    ));
    assert!(!crate::intent::continuation_resolver::immediate_prior_turn_was_clarify(
        "### LAST_TURN_FULL\n[TURN -1]\nUser: 看看那个重启脚本在不在\nAssistant: 有，路径：scripts/restart_clawd_latest.sh\n[/TURN]"
    ));
}

#[test]
fn transcript_probe_is_enabled_for_locator_only_reply_without_session_state() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(should_probe_transcript_for_clarify_fallback(
        "/tmp/device_local/logs/model_io.log",
        &snapshot,
    ));
}

#[test]
fn transcript_probe_is_skipped_when_session_state_already_exists() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: None,
            semantic_kind: None,
            source_request: "看一下那个日志最后 5 行".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    assert!(!should_probe_transcript_for_clarify_fallback(
        "/tmp/device_local/logs/model_io.log",
        &snapshot,
    ));
}

#[test]
fn transcript_probe_is_skipped_for_regular_new_request() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!should_probe_transcript_for_clarify_fallback(
        "读取 /tmp/device_local/logs/model_io.log 最后 5 行",
        &snapshot,
    ));
}

#[test]
fn transcript_probe_is_skipped_when_primary_task_prompt_exists() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Help me write a proposal".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!should_probe_transcript_for_clarify_fallback(
        "It is for executives",
        &snapshot,
    ));
}

#[test]
fn clarify_followup_routing_prompt_merges_previous_operation_for_non_locator_reply_target() {
    let merged = crate::intent::continuation_resolver::resolve_clarify_followup(
        "就在 scripts/restart_clawd_latest.sh",
        Some("[LAST_TURN_FULL]\nUser: 把那个重启脚本发给我\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]"),
        None,
        None,
        None,
    );
    match merged {
        crate::intent::continuation_resolver::ClarifyFollowupResolution::NormalizerRewrite {
            rewritten_prompt,
        } => {
            assert!(rewritten_prompt.contains("把那个重启脚本发给我"));
            assert!(rewritten_prompt.contains("就在 scripts/restart_clawd_latest.sh"));
        }
        other => panic!("expected normalizer rewrite, got {other:?}"),
    }
}

#[test]
fn clarify_followup_routing_prompt_skips_unrelated_new_request() {
    assert!(matches!(
        crate::intent::continuation_resolver::resolve_clarify_followup(
            "今天天气怎么样",
            Some(
                "[LAST_TURN_FULL]\nUser: 把那个 JSON 数组按 score 排一下并转成表格\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]"
            ),
            None,
            None,
            None,
        ),
        crate::intent::continuation_resolver::ClarifyFollowupResolution::None
    ));
}

#[test]
fn clarify_followup_resolution_disables_active_task_merge() {
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::NormalizerRewrite {
            rewritten_prompt:
                "Continue the previous request that was waiting for clarification: 看看日志最后 5 行"
                    .to_string(),
        };
    assert!(!should_apply_task_turn_merge(&resolution));
    assert!(should_apply_task_turn_merge(
        &crate::intent::continuation_resolver::ClarifyFollowupResolution::None
    ));
}

#[test]
fn task_append_merge_reuses_prior_primary_task_prompt() {
    let merged = merged_prompt_from_task_turn_analysis(
        Some("帮我写个方案"),
        None,
        "面向老板",
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskAppend),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(json!({"audience":"boss"})),
            attachment_processing_required: false,
        }),
    )
    .expect("merged prompt");
    assert!(merged.contains("帮我写个方案"));
    assert!(merged.contains("面向老板"));
    assert!(merged.contains("\"audience\":\"boss\""));
    assert!(merged.contains("append this new instruction"));
    assert!(merged.contains("Continuity rules"));
    assert!(merged.contains("Continuity does not preserve reply language"));
    assert!(merged.contains("do not preserve its question shape"));
    assert!(merged.contains("do not repeat the same clarification indefinitely"));
}

#[test]
fn task_replace_merge_discards_prior_goal() {
    let merged = merged_prompt_from_task_turn_analysis(
        Some("别写长文，先做方案"),
        None,
        "算了，改成短帖串",
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskReplace),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReplaceActive),
            should_interrupt_active_run: false,
            state_patch: Some(json!({"deliverable":"thread"})),
            attachment_processing_required: false,
        }),
    )
    .expect("merged prompt");
    assert!(merged.contains("别写长文，先做方案"));
    assert!(merged.contains("算了，改成短帖串"));
    assert!(merged.contains("\"deliverable\":\"thread\""));
    assert!(merged.contains("replace it with this new goal"));
}

#[test]
fn task_correct_merge_marks_conflicting_details_as_overrides() {
    let merged = merged_prompt_from_task_turn_analysis(
        Some("帮我写安装说明，面向 Python 3.10"),
        None,
        "不对，不是 Python 3.10，是 Python 3.11",
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(json!({"python_version":"3.11"})),
            attachment_processing_required: false,
        }),
    )
    .expect("merged prompt");
    assert!(merged.contains("Python 3.10"));
    assert!(merged.contains("Python 3.11"));
    assert!(merged.contains("\"python_version\":\"3.11\""));
    assert!(merged.contains("overrides conflicting earlier details"));
}

#[test]
fn task_append_merge_includes_recent_generated_output_when_normalizer_reuses_active() {
    let merged = merged_prompt_from_task_turn_analysis(
        Some("Write one deployment note that mentions Python 3.11"),
        Some("Deployment note: use Python 3.11."),
        "Output only that sentence",
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskAppend),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
    )
    .expect("merged prompt");
    assert!(merged.contains("Most recent generated output"));
    assert!(merged.contains("Deployment note: use Python 3.11."));
}

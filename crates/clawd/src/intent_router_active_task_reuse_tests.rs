// Active task reuse and scope-refinement tests for intent_router.

use crate::FirstLayerDecision;

use super::{
    ActFinalizeStyle, ClarifyQuestionPolicy, IntentOutputContract, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind, ScheduleKind, TargetTaskPolicy,
    TurnType,
};

#[test]
fn clarify_question_policy_defaults_to_allow_model() {
    assert_eq!(
        ClarifyQuestionPolicy::default(),
        ClarifyQuestionPolicy::AllowModel
    );
}

#[test]
fn scope_update_clarify_is_resolved_when_active_task_exists() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(
        super::should_resolve_task_scope_update_clarify_with_active_task(
            "先只看登录模块",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::Clarify,
            &IntentOutputContract::default(),
            None,
        )
    );
}

#[test]
fn scope_update_clarify_reuses_active_task_without_keyword_detector() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Help me create a rollout plan".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(
        super::should_resolve_task_scope_update_clarify_with_active_task(
            "Keep it limited to the onboarding flow",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::Clarify,
            &IntentOutputContract::default(),
            None,
        )
    );
}

#[test]
fn task_replace_clarify_is_resolved_when_active_task_exists() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a long article about RustClaw".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_resolve_task_replace_clarify_with_active_task(
        "Actually, replace it with a short thread",
        Some(&snapshot),
        Some(TurnType::TaskReplace),
        Some(TargetTaskPolicy::ReplaceActive),
        false,
        FirstLayerDecision::Clarify,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn task_replace_clarify_reuses_active_task_without_keyword_detector() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a launch memo about RustClaw".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_resolve_task_replace_clarify_with_active_task(
        "Make it a shorter internal memo instead",
        Some(&snapshot),
        Some(TurnType::TaskReplace),
        Some(TargetTaskPolicy::ReplaceActive),
        false,
        FirstLayerDecision::Clarify,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn active_task_scope_update_is_routed_back_to_direct_answer() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    assert!(super::should_route_active_task_mutation_to_direct_answer(
        "先只看登录模块",
        Some(&snapshot),
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::PlannerExecute,
        &contract,
        None,
    ));
}

#[test]
fn active_bound_path_answer_candidate_stays_direct_answer() {
    let workspace = super::test_support::make_temp_workspace_with_child(
        "active_bound_path_answer_candidate",
        "docs",
    );
    let target = workspace.join("docs").join("service_notes.md");
    std::fs::write(&target, "# Service Notes\n").expect("write target");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace.clone();
    state.skill_rt.default_locator_search_dir = workspace.clone();
    let target_text = target.display().to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("send the selected file".to_string()),
            last_primary_task_output: Some(format!("FILE:{target_text}")),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some(target_text.clone()),
            ordered_entries: vec![target_text.clone()],
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some(target_text.clone()),
            delivery_targets: vec![target_text.clone()],
            ordered_entries: vec![target_text.clone()],
            ..crate::observed_facts::ObservedFacts::default()
        }),
        active_clarify_state: None,
    };
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize = ActFinalizeStyle::ChatWrapped;
    let mut wants_file_delivery = false;
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: target_text.clone(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_active_bound_path_answer_candidate_direct_repair(
        &state,
        Some(&snapshot),
        &target_text,
        false,
        ScheduleKind::None,
        &mut decision,
        &mut finalize,
        &mut wants_file_delivery,
        &mut contract,
    );

    assert_eq!(repair, Some("active_bound_path_answer_candidate_direct"));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize, ActFinalizeStyle::Plain);
    assert!(!wants_file_delivery);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(contract.locator_hint.is_empty());
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn missing_active_task_reuse_policy_is_repaired_to_clarify() {
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_missing_active_task_reuse_clarify(
        "",
        None,
        None,
        Some(TargetTaskPolicy::ReuseActive),
        None,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(reason, Some("missing_active_task_reuse_requires_clarify"));
    assert!(needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn missing_active_task_reuse_policy_preserves_standalone_answer_candidate() {
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_missing_active_task_reuse_clarify(
        "",
        None,
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
        Some("Use Python 3.10 for deployment."),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(reason, None);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
}

#[test]
fn missing_active_task_reuse_policy_preserves_current_task_execution_contract() {
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/tmp/work/document".to_string(),
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_missing_active_task_reuse_clarify(
        "",
        None,
        Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::ReuseActive),
        None,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(reason, None);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::FileNames);
    assert_eq!(contract.locator_hint, "/tmp/work/document");
}

#[test]
fn missing_active_task_reuse_policy_preserves_status_query_execution_contract() {
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::ServiceStatus,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_missing_active_task_reuse_clarify(
        "",
        None,
        Some(TurnType::StatusQuery),
        Some(TargetTaskPolicy::ReuseActive),
        None,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(reason, None);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ServiceStatus);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn structured_replacement_patch_repairs_active_task_correction_metadata() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "Write a short setup checklist for RustClaw".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let patch = serde_json::json!({
        "replacements": [
            {"old": "Python 3.10", "new": "Python 3.11"}
        ]
    });
    let mut turn_type = None;
    let mut target_task_policy = None;
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_structured_patch_repair(
        "Use Python 3.11 instead of Python 3.10.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, Some("active_task_structured_patch_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskCorrect));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn ordered_entry_execution_patch_preserves_planner_contract() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("List the first five log files".to_string()),
            last_primary_task_output: Some("act_plan.log\nclawd-dev.log".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "List the first five log files".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/home/guagua/rustclaw/logs".to_string()),
            ordered_entries: vec!["act_plan.log".to_string(), "clawd-dev.log".to_string()],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let patch = serde_json::json!({
        "ordered_entry_ref": {"index": 1, "index_base": 1},
        "active_task_scope": {
            "operation": "tail_last_n_lines",
            "n": 3
        }
    });
    let mut turn_type = Some(TurnType::TaskAppend);
    let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_structured_patch_repair(
        "Use the selected ordered entry and read a bounded slice.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskAppend));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(!needs_clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn structured_patch_repair_does_not_clear_execution_failed_step_contract() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "Run echo BEFORE, then a missing command, then echo AFTER".to_string(),
            ),
            last_primary_task_output: Some(
                "Step 2 failed with command_not_found; echo AFTER remains".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let patch = serde_json::json!({
        "conditional_step_update": {
            "step_to_modify": 3,
            "original_command": "echo AFTER_CHANGE_OLD",
            "replacement_command": "echo AFTER_CHANGE_NEW",
            "trigger_condition": "user_says_continue_after_failure"
        }
    });
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExecutionFailedStep,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_structured_patch_repair(
        "Run echo BEFORE_CHANGE, then definitely_missing_command, then echo AFTER_CHANGE_OLD; if I later continue, replace the last step.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskRequest));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(!needs_clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExecutionFailedStep
    );
}

#[test]
fn structured_patch_repair_does_not_override_explicit_filename_target() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a release note".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let patch = serde_json::json!({
        "replacements": [
            {"old": "Python 3.10", "new": "Python 3.11"}
        ]
    });
    let mut turn_type = None;
    let mut target_task_policy = None;
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_structured_patch_repair(
        "In README.md, replace Python 3.10 with Python 3.11.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, None);
    assert_eq!(target_task_policy, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
}

#[test]
fn scalar_patch_with_locator_hint_requires_active_binding_for_repair() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
            last_primary_task_output: Some(
                "RustClaw Release Notes - Your Quick Checklist".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let patch = serde_json::json!({"release_notes_python_version": "Python 3.11"});
    let mut turn_type = None;
    let mut target_task_policy = None;
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "release notes".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_structured_patch_repair(
        "Use Python 3.11 instead of Python 3.10.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, None);
    assert_eq!(target_task_policy, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_hint, "release notes");

    let mut turn_type = Some(TurnType::TaskCorrect);
    let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "release notes".to_string(),
        self_extension: Default::default(),
    };
    let reason = super::apply_active_task_structured_patch_repair(
        "Use Python 3.11 instead of Python 3.10.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, Some("active_task_structured_patch_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskCorrect));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert!(!contract.requires_content_evidence);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn standalone_execution_target_misroute_is_repaired_to_active_scope_update() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "current workspace".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "先只看登录模块",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
        false,
    );

    assert_eq!(reason, Some("active_task_scope_refinement_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskScopeUpdate));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn scope_refinement_repair_detaches_from_structured_active_target() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("GET http://127.0.0.1:8787/v1/health".to_string()),
            last_primary_task_output: Some("Service status: reachable (HTTP 200).".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "GET http://127.0.0.1:8787/v1/health".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("http://127.0.0.1:8787/v1/health".to_string()),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "current workspace".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "A concept label without a concrete target.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
        false,
    );

    assert_eq!(
        reason,
        Some("active_task_scope_refinement_detached_from_structured_anchor")
    );
    assert_eq!(turn_type, None);
    assert_eq!(target_task_policy, None);
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn active_ordered_scalar_path_without_ref_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "find matching files".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/rustclaw/fuzzy_top3".to_string()),
            ordered_entries: vec![
                "abcd_report.md".to_string(),
                "my_abcd.txt".to_string(),
                "x_abcd_log.txt".to_string(),
            ],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_ordered_scalar_path_chat_repair(
        Some(&snapshot),
        None,
        "",
        false,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(
        reason,
        Some("active_ordered_scalar_path_chat_repair_without_structured_ref")
    );
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn active_ordered_scalar_path_without_ref_repairs_planner_to_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "find matching files".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/rustclaw/fuzzy_top3".to_string()),
            ordered_entries: vec![
                "abcd_report.md".to_string(),
                "my_abcd.txt".to_string(),
                "x_abcd_log.txt".to_string(),
            ],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_ordered_scalar_path_chat_repair(
        Some(&snapshot),
        None,
        "",
        false,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(
        reason,
        Some("active_ordered_scalar_path_chat_repair_without_structured_ref")
    );
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn active_observed_output_summary_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_output: Some(r#"{"phase":"loop_done","tool_calls":1}"#.to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read recent log tail".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/rustclaw/logs/act_plan.log".to_string()),
            source_task_id: "task-read".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExcerptKindJudgment,
        locator_hint: "/tmp/rustclaw/logs/act_plan.log".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_observed_output_chat_repair(
        "one sentence status judgment",
        Some(&snapshot),
        Some(TurnType::TaskRequest),
        None,
        false,
        false,
        ScheduleKind::None,
        None,
        false,
        "",
        false,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(reason, Some("active_observed_output_chat_repair"));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn active_observed_output_chinese_category_judgment_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_output: Some(
                "2026-05-26T13:57:59Z WARN stage=cleanup\n2026-05-26T13:58:00Z INFO stage=response"
                    .to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "看最后一个最后 2 行".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/rustclaw/logs/clawd.nl-focus.log".to_string()),
            source_task_id: "task-read-log-tail".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExcerptKindJudgment,
        locator_hint: "/tmp/rustclaw/logs/clawd.nl-focus.log".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_observed_output_chat_repair(
        "一句话说它更像日志还是清单",
        Some(&snapshot),
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        false,
        ScheduleKind::None,
        None,
        false,
        "",
        false,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(reason, Some("active_observed_output_chat_repair"));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn active_observed_output_conversation_judgment_without_fresh_evidence_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_output: Some(
                "2026-05-26T13:57:59Z INFO stage=response message=ok".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read log tail".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/rustclaw/logs/clawd.nl-focus.log".to_string()),
            source_task_id: "task-read".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "/tmp/rustclaw/logs/clawd.nl-focus.log".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_observed_output_chat_repair(
        "one sentence category judgment for the latest observed output",
        Some(&snapshot),
        Some(TurnType::TaskRequest),
        None,
        false,
        false,
        ScheduleKind::None,
        None,
        false,
        "",
        false,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(reason, Some("active_observed_output_chat_repair"));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn scope_refinement_repair_preserves_current_request_workspace_child_locator() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Summarize the previous document.".to_string()),
            last_primary_task_output: Some("A previous summary.".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "Read README and summarize it.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
        true,
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskRequest));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(needs_clarify);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn scope_refinement_repair_does_not_override_explicit_locator() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "UI/src".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "先只看 UI/src",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
        false,
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskCorrect));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(needs_clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "UI/src");
}

#[test]
fn scope_refinement_repair_preserves_fresh_document_heading_contract() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "Read the previously selected local document.".to_string(),
            ),
            last_primary_task_output: Some("Service Notes".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "Read selected file title".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some(
                "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
            ),
            source_task_id: "task-old-read".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let state_patch = serde_json::json!({
        "alias_bindings": [
            {
                "alias": "doc_alias",
                "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            }
        ]
    });
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::DocumentHeading,
        locator_hint: "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            .to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "Read only the selected document heading.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&state_patch),
        false,
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskRequest));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(!needs_clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::DocumentHeading);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
}

#[test]
fn scope_refinement_repair_preserves_standalone_observation_contract() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("生成一个 JSON 文件".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "检查当前运行环境并只返回关键值",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
        false,
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskRequest));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(!needs_clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn active_task_scope_update_en_remains_direct_answer_from_chat_wrapped_execution() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Help me create a test plan".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_route_active_task_mutation_to_direct_answer(
        "Only focus on the login module first",
        Some(&snapshot),
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::PlannerExecute,
        &IntentOutputContract::default(),
        None,
    ));
}

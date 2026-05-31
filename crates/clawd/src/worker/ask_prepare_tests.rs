use super::{
    active_clarify_existing_workspace_locator_reply, active_clarify_run_control_prompt,
    bind_ordered_entry_reference_from_active_frame, merged_prompt_from_task_turn_analysis,
    preserve_active_clarify_output_contract_for_locator_reply,
    promote_active_clarify_locator_reply_to_execute,
    repair_scalar_field_value_contract_for_locator_reply,
    repair_structural_file_delivery_resolution, should_apply_task_turn_merge,
    should_probe_transcript_for_clarify_fallback, task_turn_merge_prior_context,
};

use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

fn make_temp_root(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rustclaw_ask_prepare_{label}_{}_{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&path).expect("temp root");
    path
}

#[test]
fn binding_context_marks_recent_failed_candidate_without_mutating_source() {
    let binding = crate::intent::resume_policy::ResumeContextBinding {
        source: crate::intent::resume_policy::ResumeContextSource::RecentFailedCandidate,
        resume_context: json!({"resume_context_id":"ctx-1"}),
        failed_ts: Some(42),
        has_newer_successful_ask_after_failed_task: true,
    };
    let value = crate::intent::resume_policy::binding_context_json("manual", false, Some(&binding));
    assert_eq!(
        value.get("resume_context_source").and_then(|v| v.as_str()),
        Some("recent_failed_resume_candidate")
    );
    assert_eq!(
        value
            .get("is_resume_continue_source")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .get("has_newer_successful_ask_after_failed_task")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn task_turn_merge_prior_prefers_active_clarify_over_stale_primary_task() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "读取 scripts/nl_tests/fixtures/device_local/package.json 的 name 字段".to_string(),
            ),
            last_primary_task_output: Some("rustclaw-nl-fixture".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供要发送的文件路径或文件名。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: true,
            output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
            semantic_kind: None,
            source_request: "把那个最大的发给我。".to_string(),
            source_task_id: "task-clarify".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_followup_frame: None,
        active_observed_facts: None,
    };

    let (prompt, output) = task_turn_merge_prior_context(&snapshot);

    assert_eq!(prompt, Some("把那个最大的发给我。"));
    assert_eq!(output, Some("请提供要发送的文件路径或文件名。"));
}

#[test]
fn active_clarify_accepts_existing_workspace_child_as_locator_reply() {
    let root = make_temp_root("clarify_existing_child");
    std::fs::create_dir_all(root.join("scripts")).expect("scripts dir");
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供具体目标或路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: None,
            source_request: "数一下那个目录里有多少个直接子项，只输出数字".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };

    let resolution =
        active_clarify_existing_workspace_locator_reply(&root, &root, "scripts", &snapshot)
            .expect("existing workspace child should fill locator clarify");

    match resolution {
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            hit,
        ) => {
            assert_eq!(hit.current_user_text, "scripts");
            assert!(hit
                .resolved_intent
                .contains("数一下那个目录里有多少个直接子项，只输出数字"));
        }
        other => panic!("expected locator rewrite, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_clarify_existing_locator_reply_requires_existing_path() {
    let root = make_temp_root("clarify_missing_child");
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Target?".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: None,
            source_request: "Count that directory".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };

    assert!(active_clarify_existing_workspace_locator_reply(
        &root,
        &root,
        "missing_child",
        &snapshot
    )
    .is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_clarify_resolves_unique_nested_filename_reply() {
    let root = make_temp_root("clarify_unique_nested_file");
    std::fs::create_dir_all(root.join("scripts")).expect("scripts dir");
    std::fs::write(root.join("scripts").join("restart_once.sh"), "#!/bin/sh\n")
        .expect("fixture file");
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Target?".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: Some(
                crate::OutputSemanticKind::ExistenceWithPath
                    .as_str()
                    .to_string(),
            ),
            source_request: "检查那个重启脚本在不在".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };

    let resolution =
        active_clarify_existing_workspace_locator_reply(&root, &root, "restart_once.sh", &snapshot)
            .expect("unique nested filename should fill locator clarify");

    match resolution {
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            hit,
        ) => {
            assert_eq!(hit.current_user_text, "scripts/restart_once.sh");
            assert!(hit.resolved_intent.contains("scripts/restart_once.sh"));
        }
        other => panic!("expected locator rewrite, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_clarify_accepts_locator_reply_without_explicit_output_contract() {
    let root = make_temp_root("clarify_plain_locator");
    std::fs::create_dir_all(root.join("scripts")).expect("scripts dir");
    std::fs::write(
        root.join("scripts").join("restart_clawd_latest.sh"),
        "#!/bin/sh\n",
    )
    .expect("fixture file");
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供具体目标或路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: None,
            semantic_kind: None,
            source_request: "看看那个重启脚本在不在".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };

    let resolution = active_clarify_existing_workspace_locator_reply(
        &root,
        &root,
        "restart_clawd_latest.sh",
        &snapshot,
    )
    .expect("plain locator clarify should fill an existing unique workspace entry");

    match resolution {
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            hit,
        ) => {
            assert_eq!(hit.current_user_text, "scripts/restart_clawd_latest.sh");
            assert!(hit.resolved_intent.contains("看看那个重启脚本在不在"));
        }
        other => panic!("expected locator rewrite, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_clarify_does_not_guess_ambiguous_nested_filename_reply() {
    let root = make_temp_root("clarify_ambiguous_nested_file");
    std::fs::create_dir_all(root.join("a")).expect("dir a");
    std::fs::create_dir_all(root.join("b")).expect("dir b");
    std::fs::write(root.join("a").join("same.md"), "a").expect("fixture a");
    std::fs::write(root.join("b").join("same.md"), "b").expect("fixture b");
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Target?".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: Some(
                crate::OutputSemanticKind::ExistenceWithPath
                    .as_str()
                    .to_string(),
            ),
            source_request: "检查那个文件在不在".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };

    assert!(
        active_clarify_existing_workspace_locator_reply(&root, &root, "same.md", &snapshot)
            .is_none()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_clarify_run_control_prompt_blocks_unrelated_alias_selection() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::RunControl),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReplaceActive),
        should_interrupt_active_run: true,
        state_patch: None,
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "甲文件".to_string(),
                target: "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
                    .to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供要发送的文件路径或文件名。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: true,
            output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
            semantic_kind: None,
            source_request: "把那个最大的发给我。".to_string(),
            source_task_id: "task-clarify".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_followup_frame: None,
        active_observed_facts: None,
    };

    let prompt = active_clarify_run_control_prompt(
        &route,
        Some(&turn_analysis),
        &snapshot,
        "停一下，不要发文件，改为只告诉我你需要我确认哪个文件。",
    )
    .expect("clarify control prompt");

    assert!(prompt.contains("Missing information to confirm"));
    assert!(prompt.contains("Candidate targets from that clarification only:\n<none>"));
    assert!(!prompt.contains("release_checklist.md"));
}

#[test]
fn runtime_resume_binding_is_disabled_when_normalizer_rejects_resume() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "list current workspace".to_string(),
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
    };
    let binding = crate::intent::resume_policy::ResumeContextBinding {
        source: crate::intent::resume_policy::ResumeContextSource::RecentFailedCandidate,
        resume_context: json!({"resume_context_id":"ctx-2"}),
        failed_ts: Some(7),
        has_newer_successful_ask_after_failed_task: false,
    };
    assert!(
        crate::intent::resume_policy::select_resume_runtime_binding(&route, Some(&binding))
            .is_none()
    );
}

#[test]
fn clarify_locator_reply_preserves_prior_content_excerpt_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "读取文件最后 10 行并发送内容".to_string(),
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
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "/tmp/model_io.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "请提供日志路径".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: false,
        output_shape: None,
        semantic_kind: Some(
            crate::OutputSemanticKind::ContentExcerptSummary
                .as_str()
                .to_string(),
        ),
        source_request: "看下那个最近 10 行".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(clarify_state),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent: "Continue...".to_string(),
                prior_user_text: "看下那个最近 10 行".to_string(),
                current_user_text: "/tmp/model_io.log".to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    preserve_active_clarify_output_contract_for_locator_reply(&mut route, &resolution, &snapshot);

    assert!(!route.wants_file_delivery);
    assert!(!route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::None
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert!(route
        .route_reason
        .contains("preserve_active_clarify_output_contract"));
}

#[test]
fn clarify_locator_reply_keeps_current_file_delivery_over_weak_prior_shape() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Deliver the existing file as a file token".to_string(),
        needs_clarify: false,
        route_reason: "llm_semantic_contract_repair".to_string(),
        route_confidence: Some(0.95),
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
            locator_hint: "scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
                .to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Which configuration file?".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: None,
            source_request: "Send that local config without pasting the body".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent:
                    "Continue the previous request that was waiting for clarification: Send that local config without pasting the body\nUser now provides the missing target/content: scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
                        .to_string(),
                prior_user_text: "Send that local config without pasting the body".to_string(),
                current_user_text:
                    "scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
                        .to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    preserve_active_clarify_output_contract_for_locator_reply(&mut route, &resolution, &snapshot);

    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert!(route
        .route_reason
        .contains("keep_current_file_delivery_over_weak_active_clarify_shape"));
}

#[test]
fn clarify_locator_reply_promotes_bare_path_back_to_execution() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "scripts/nl_tests/fixtures/device_local/logs/model_io.log".to_string(),
        needs_clarify: true,
        route_reason: "bare_path_no_verb".to_string(),
        route_confidence: Some(0.8),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: "What would you like me to do with the file?".to_string(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "请提供日志路径".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: false,
        output_shape: None,
        semantic_kind: Some(
            crate::OutputSemanticKind::ContentExcerptSummary
                .as_str()
                .to_string(),
        ),
        source_request: "看看那个模型日志最后 5 行".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(clarify_state),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent:
                    "Continue the previous request that was waiting for clarification: 看看那个模型日志最后 5 行\nUser now provides the missing target/content: scripts/nl_tests/fixtures/device_local/logs/model_io.log"
                        .to_string(),
                prior_user_text: "看看那个模型日志最后 5 行".to_string(),
                current_user_text: "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
                    .to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    preserve_active_clarify_output_contract_for_locator_reply(&mut route, &resolution, &snapshot);
    promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert!(route.clarify_question.is_empty());
    assert_eq!(
        route.output_contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/logs/model_io.log"
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert!(route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("active_clarify_locator_reply_execute"));
}

#[test]
fn clarify_archive_locator_reply_restores_unpack_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Read archive content".to_string(),
        needs_clarify: false,
        route_reason: "User supplied missing archive path".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let destination =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/tmp/clarify_unpack_case";
    let archive =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let source_request = format!("extract missing archive into {destination}");
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "请提供压缩包路径".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: false,
        output_shape: None,
        semantic_kind: None,
        source_request: source_request.clone(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(clarify_state),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent: format!(
                    "Continue previous archive unpack request with archive: {archive}"
                ),
                prior_user_text: source_request,
                current_user_text: archive.to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    preserve_active_clarify_output_contract_for_locator_reply(&mut route, &resolution, &snapshot);
    promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ArchiveUnpack
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::OneSentence
    );
    assert!(route.output_contract.requires_content_evidence);
    assert!(!route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.locator_hint,
        format!("{archive} | {destination}")
    );
    assert!(route
        .route_reason
        .contains("active_clarify_archive_unpack_pair_repaired"));
}

#[test]
fn clarify_structured_payload_reply_is_not_rewritten_as_locator() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Transform inline structured data into a table".to_string(),
        needs_clarify: false,
        route_reason: "Inline structured data transform".to_string(),
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
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "请提供 JSON 数组".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: false,
        output_shape: Some(crate::OutputResponseShape::Strict.as_str().to_string()),
        semantic_kind: None,
        source_request: "把那个 JSON 数组按 score 排一下并转成表格".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(clarify_state),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent: "Continue the previous request with provided structured payload."
                    .to_string(),
                prior_user_text: "把那个 JSON 数组按 score 排一下并转成表格".to_string(),
                current_user_text: r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#
                    .to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    preserve_active_clarify_output_contract_for_locator_reply(&mut route, &resolution, &snapshot);
    promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    assert!(!route
        .route_reason
        .contains("active_clarify_locator_reply_execute"));
    assert!(!route
        .route_reason
        .contains("preserve_active_clarify_output_contract"));
}

#[test]
fn scalar_field_value_contract_repair_clears_structured_keys_semantic_kind() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Extract only the field value from a structured file".to_string(),
        needs_clarify: false,
        route_reason: "llm_semantic_contract_repair:contract_valid_minor_repair_fields_only"
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
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::StructuredKeys,
            locator_hint: "package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };

    repair_scalar_field_value_contract_for_locator_reply(&mut route);

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route
        .route_reason
        .contains("scalar_field_value_contract_repair"));
}

#[test]
fn clarify_locator_reply_preserves_prior_file_delivery_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "README.md".to_string(),
        needs_clarify: true,
        route_reason: "bare_path_no_verb".to_string(),
        route_confidence: Some(0.8),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: "What file?".to_string(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
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
            source_request: "Send me the file".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent:
                    "Continue the previous request that was waiting for clarification: Send me the file\nUser now provides the missing target/content: README.md"
                        .to_string(),
                prior_user_text: "Send me the file".to_string(),
                current_user_text: "README.md".to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert_eq!(route.output_contract.locator_hint, "README.md");
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Filename
    );
}

#[test]
fn clarify_locator_reply_injects_locator_into_existing_execute_route() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "read and deliver config file".to_string(),
        needs_clarify: false,
        route_reason: "semantic_contract_requires_evidence".to_string(),
        route_confidence: Some(1.0),
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
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::FileSingle,
            semantic_kind: crate::OutputSemanticKind::FilePaths,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
            ..Default::default()
        },
    };
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
            source_request: "Send that config file".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent:
                    "Continue the previous request that was waiting for clarification: Send that config file\nUser now provides the missing target/content: /tmp/app_config.toml"
                        .to_string(),
                prior_user_text: "Send that config file".to_string(),
                current_user_text: "/tmp/app_config.toml".to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert_eq!(route.output_contract.locator_hint, "/tmp/app_config.toml");
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
}

#[test]
fn clarify_locator_reply_does_not_promote_stale_prior_request() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "/tmp/a.log".to_string(),
        needs_clarify: true,
        route_reason: "bare_path_no_verb".to_string(),
        route_confidence: Some(0.8),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: "path?".to_string(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "path?".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: None,
            semantic_kind: None,
            source_request: "上一轮请求".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent: "Continue...".to_string(),
                prior_user_text: "另一轮请求".to_string(),
                current_user_text: "/tmp/a.log".to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

    assert_eq!(route.ask_mode, crate::AskMode::clarify());
    assert!(route.needs_clarify);
}

#[test]
fn clarify_locator_reply_drops_untrusted_current_semantic_when_prior_only_shape() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Continue the prior task using scripts".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
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
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_hint: "scripts".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
            ..Default::default()
        },
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Provide the missing target path.".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: None,
            source_request: "Count direct children in the target directory.".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent:
                    "Continue the previous request that was waiting for clarification.".to_string(),
                prior_user_text: "Count direct children in the target directory.".to_string(),
                current_user_text: "scripts".to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    preserve_active_clarify_output_contract_for_locator_reply(&mut route, &resolution, &snapshot);

    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("drop_untrusted_locator_reply_semantic_kind"));
    assert!(route
        .route_reason
        .contains("preserve_active_clarify_output_contract"));
}

#[test]
fn clarify_locator_reply_preserves_prior_scalar_path_contract_without_delivery() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "在目录 fixtures/stem_unique 中查找 abcd".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
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
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "fixtures/stem_unique".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "请提供要搜索的目录或目标文件的具体路径。".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: false,
        output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
        semantic_kind: Some(
            crate::OutputSemanticKind::ScalarPathOnly
                .as_str()
                .to_string(),
        ),
        source_request: "去那个 stem_unique 目录里找 abcd，只输出路径".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(clarify_state),
        active_observed_facts: None,
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent: "Continue...".to_string(),
                prior_user_text: "去那个 stem_unique 目录里找 abcd，只输出路径".to_string(),
                current_user_text: "fixtures/stem_unique".to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    preserve_active_clarify_output_contract_for_locator_reply(&mut route, &resolution, &snapshot);

    assert!(!route.wants_file_delivery);
    assert!(!route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ScalarPathOnly
    );
    assert!(route
        .route_reason
        .contains("preserve_active_clarify_output_contract"));
}

#[test]
fn file_delivery_with_structured_locator_is_preserved() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
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
fn unresolved_file_delivery_without_locator_requires_clarify() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
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

    assert!(route.needs_clarify);
    assert!(route.is_clarify_gate());
    assert!(!route.wants_file_delivery);
    assert!(!route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::None
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.clarify_question.contains("文件路径"));
    assert!(route
        .route_reason
        .contains("unresolved_file_delivery_requires_clarify"));
}

#[test]
fn generated_file_delivery_without_locator_can_choose_runtime_target() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "create a shell script, save it, and deliver the generated file"
            .to_string(),
        needs_clarify: true,
        route_reason: String::new(),
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
fn structurally_resolved_file_delivery_binds_recent_read_target_without_text_match() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
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
    assert!(route.is_execute_gate());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert_eq!(
        route.output_contract.locator_hint,
        "/tmp/README.md".to_string()
    );
    assert!(route.resolved_intent.contains("/tmp/README.md"));
    assert!(route
        .route_reason
        .contains("structural_file_delivery_bound_to_recent_read_target"));
}

#[test]
fn ordered_entry_reference_binds_third_delivery_from_active_frame() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
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
        Some(&analysis)
    ));

    assert_eq!(route.output_contract.locator_hint, "logs/clawd.run.log");
    assert!(route
        .route_reason
        .contains("ordered_entry_reference_bound_from_active_frame"));
    assert!(route.resolved_intent.contains("logs/clawd.run.log"));
}

#[test]
fn ordered_entry_reference_binds_previous_from_selected_entry() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
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
        Some(&analysis)
    ));

    assert_eq!(route.output_contract.locator_hint, "logs/clawd.log");
    assert!(route.resolved_intent.contains("logs/clawd.log"));
}

#[test]
fn ordered_entry_reference_binds_scalar_path_from_active_frame() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "return only the selected path".to_string(),
        needs_clarify: false,
        route_reason: "normalizer selected an active ordered entry".to_string(),
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
        Some(&analysis)
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
fn filename_only_output_patch_clears_file_delivery_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
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

    super::clear_file_delivery_contract_for_filename_only(&mut route, Some(&analysis));

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
        "算了，改成 X thread",
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
    assert!(merged.contains("算了，改成 X thread"));
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

use super::{
    active_clarify_existing_workspace_locator_reply, active_clarify_locator_reply_fast_path_route,
    active_clarify_run_control_prompt, active_clarify_state_has_structural_binding_contract,
    append_active_delivery_content_target_token, bind_content_read_to_active_delivery_target,
    bind_ordered_entry_reference_from_active_frame, merged_prompt_from_task_turn_analysis,
    preserve_active_clarify_output_contract_for_locator_reply,
    preserve_locator_reply_runtime_intent, promote_active_clarify_locator_reply_to_execute,
    promote_active_clarify_structured_payload_reply_to_execute,
    repair_scalar_field_value_contract_for_locator_reply,
    repair_structural_file_delivery_resolution, should_apply_task_turn_merge,
    should_probe_transcript_for_clarify_fallback, task_turn_merge_prior_context,
};

use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "ask_prepare_tests/followup_delivery.rs"]
mod followup_delivery;

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

fn test_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: "ask-prepare-test".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("ask-prepare-user".to_string()),
        channel: "api".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
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

    let (prompt, output) = task_turn_merge_prior_context(&snapshot, Some("latest visible reply"));

    assert_eq!(prompt.as_deref(), Some("把那个最大的发给我。"));
    assert_eq!(output.as_deref(), Some("请提供要发送的文件路径或文件名。"));
}

#[test]
fn task_turn_merge_prior_prefers_primary_output_over_unstructured_latest_reply() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("把那个最大的发给我。".to_string()),
            last_primary_task_output: Some(
                "就是 `/home/guagua/rustclaw`，你想确认哪个？".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_clarify_state: None,
        active_followup_frame: None,
        active_observed_facts: None,
    };
    let latest = "需要更多信息才能确定——请告诉我是哪个文件需要确认。";

    let (prompt, output) = task_turn_merge_prior_context(&snapshot, Some(latest));
    let merged = merged_prompt_from_task_turn_analysis(
        prompt.as_deref(),
        output.as_deref(),
        "刚才那句如果太生硬，就更口语化一点，只输出新版。",
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
    )
    .expect("merged prompt");

    assert!(merged.contains("Most recent generated output"));
    assert!(merged.contains("就是 `/home/guagua/rustclaw`"));
    assert!(!merged.contains(latest));
}

#[test]
fn task_turn_merge_prior_ignores_side_question_latest_reply() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a short release note for RustClaw.".to_string()),
            last_primary_task_output: Some(
                "1. Connect RustClaw to your chat app.\n2. Manage tasks in one place.\n3. Use Python 3.11 support."
                    .to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_clarify_state: None,
        active_followup_frame: None,
        active_observed_facts: None,
    };
    let latest = "SQLite is a lightweight file-based SQL database engine.";

    let (prompt, output) = task_turn_merge_prior_context(&snapshot, Some(latest));
    let merged = merged_prompt_from_task_turn_analysis(
        prompt.as_deref(),
        output.as_deref(),
        "Return to the checklist and output only three short bullets.",
        Some(&crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "primary_task_update": "patch"
            })),
            attachment_processing_required: false,
        }),
    )
    .expect("merged prompt");

    assert!(merged.contains("Write a short release note for RustClaw."));
    assert!(merged.contains("Use Python 3.11 support."));
    assert!(!merged.contains("SQLite"));
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
fn active_clarify_locator_fast_path_reuses_file_delivery_contract() {
    let root = make_temp_root("clarify_fast_path_delivery");
    std::fs::create_dir_all(root.join("configs")).expect("configs dir");
    std::fs::write(root.join("configs/config.toml"), "app_name = \"demo\"").expect("config file");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let task = test_task();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供要发送的文件路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: true,
            output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
            semantic_kind: None,
            source_request: "把那个配置文件发给我".to_string(),
            source_task_id: "task-clarify".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution = active_clarify_existing_workspace_locator_reply(
        &root,
        &root,
        "configs/config.toml",
        &snapshot,
    )
    .expect("existing config path should resolve");

    let route = active_clarify_locator_reply_fast_path_route(&state, &task, &snapshot, &resolution)
        .expect("active clarify locator reply should use fast path");

    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert_eq!(route.output_contract.locator_hint, "configs/config.toml");
    assert!(route
        .route_reason
        .contains("active_clarify_locator_reply_fast_path"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_clarify_locator_fast_path_reuses_scalar_field_contract() {
    let root = make_temp_root("clarify_fast_path_scalar_field");
    std::fs::write(root.join("package.json"), r#"{"name":"rustclaw"}"#).expect("package json");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let task = test_task();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供要读取的文件路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: Some(
                crate::OutputSemanticKind::StructuredKeys
                    .as_str()
                    .to_string(),
            ),
            source_request: "读一下那个文件里的名字字段，只输出值".to_string(),
            source_task_id: "task-clarify".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution =
        active_clarify_existing_workspace_locator_reply(&root, &root, "package.json", &snapshot)
            .expect("existing package path should resolve");

    let route = active_clarify_locator_reply_fast_path_route(&state, &task, &snapshot, &resolution)
        .expect("active clarify scalar locator reply should use fast path");

    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert!(!route.wants_file_delivery);
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert_eq!(route.output_contract.locator_hint, "package.json");
    assert!(route
        .route_reason
        .contains("active_clarify_fast_path_scalar_field_value_contract_repair"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn active_clarify_locator_fast_path_preserves_existence_contract() {
    let root = make_temp_root("clarify_fast_path_existence");
    std::fs::create_dir_all(root.join("scripts")).expect("scripts dir");
    std::fs::write(root.join("scripts/restart_clawd_latest.sh"), "#!/bin/sh\n")
        .expect("restart script");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let task = test_task();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供具体要查找的名称、目录或路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(crate::OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: Some(
                crate::OutputSemanticKind::ExistenceWithPath
                    .as_str()
                    .to_string(),
            ),
            source_request: "看看那个重启脚本在不在".to_string(),
            source_task_id: "task-clarify".to_string(),
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
    .expect("existing nested script should resolve");

    let route = active_clarify_locator_reply_fast_path_route(&state, &task, &snapshot, &resolution)
        .expect("active clarify existence locator reply should use fast path");

    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ExistenceWithPath
    );
    assert_eq!(
        route.output_contract.locator_hint,
        "scripts/restart_clawd_latest.sh"
    );
    assert!(!route
        .route_reason
        .contains("active_clarify_fast_path_scalar_field_value_contract_repair"));
    assert!(route
        .route_reason
        .contains("preserve_active_clarify_output_contract"));
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
fn active_clarify_locator_reply_preserves_existence_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Read the provided file target.".to_string(),
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
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Filename,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "restart_once.sh".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "Target?".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: false,
        output_shape: Some(crate::OutputResponseShape::Strict.as_str().to_string()),
        semantic_kind: Some(
            crate::OutputSemanticKind::ExistenceWithPath
                .as_str()
                .to_string(),
        ),
        source_request: "Check whether the restart script exists.".to_string(),
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
                resolved_intent: "Continue the previous existence check for restart_once.sh."
                    .to_string(),
                prior_user_text: "Check whether the restart script exists.".to_string(),
                current_user_text: "restart_once.sh".to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        );

    preserve_active_clarify_output_contract_for_locator_reply(&mut route, &resolution, &snapshot);
    promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ExistenceWithPath
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Strict
    );
    assert_eq!(route.output_contract.locator_hint, "restart_once.sh");
    assert!(route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("preserve_active_clarify_output_contract"));
    assert!(route
        .route_reason
        .contains("active_clarify_locator_reply_execute"));
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
fn active_clarify_candidate_only_state_is_not_fast_path_contract() {
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "Which README file should I read?".to_string(),
        candidate_targets: vec!["README.md".to_string(), "README.txt".to_string()],
        delivery_required: false,
        output_shape: None,
        semantic_kind: None,
        source_request: "Can you read that README header and summarize it?".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };

    assert!(
        !active_clarify_state_has_structural_binding_contract(&clarify_state),
        "candidate lists alone are clarify context, not an executable contract"
    );
}

#[test]
fn active_clarify_candidate_only_locator_reply_defers_fast_path() {
    let root = make_temp_root("clarify_candidate_only_fast_path");
    std::fs::write(root.join("README.md"), "# Demo\n").expect("fixture file");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let task = test_task();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Which README file should I read?".to_string(),
            candidate_targets: vec!["README.md".to_string(), "README.txt".to_string()],
            delivery_required: false,
            output_shape: None,
            semantic_kind: None,
            source_request: "Can you read that README header and summarize it?".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let resolution =
        active_clarify_existing_workspace_locator_reply(&root, &root, "README.md", &snapshot)
            .expect("candidate-only state should still recognize locator replies");

    assert!(
        active_clarify_locator_reply_fast_path_route(&state, &task, &snapshot, &resolution)
            .is_none(),
        "candidate-only state should go back through normal routing, not execute with an empty contract"
    );

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
fn locator_reply_runtime_intent_preserves_prior_operation_for_executable_route() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Read the TOML configuration file at the given path".to_string(),
        needs_clarify: false,
        route_reason: "normalizer_path_locator".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
                .to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let resolution =
        crate::intent::continuation_resolver::ClarifyFollowupResolution::LocatorReplyRewrite(
            crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent:
                    "Continue the previous resolved request by applying the same operation to the provided target or content.\nPrevious user request: 去那个配置里找 app.name，只把值给我\nProvided target or content: scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
                        .to_string(),
                prior_user_text: "去那个配置里找 app.name，只把值给我".to_string(),
                current_user_text:
                    "scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
                        .to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::FollowupLocatorReply,
            },
        );

    preserve_locator_reply_runtime_intent(&mut route, &resolution);

    assert!(route.resolved_intent.contains("app.name"));
    assert!(route
        .resolved_intent
        .contains("scripts/nl_tests/fixtures/device_local/configs/app_config.toml"));
    assert!(route
        .route_reason
        .contains("preserve_locator_reply_runtime_intent"));
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
                    "Continue the previous request that was waiting for clarification: Send that local config without pasting the body\nUser now provides the missing target or content: scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
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
                    "Continue the previous request that was waiting for clarification: 看看那个模型日志最后 5 行\nUser now provides the missing target or content: scripts/nl_tests/fixtures/device_local/logs/model_io.log"
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
    promote_active_clarify_structured_payload_reply_to_execute(&mut route, &resolution, &snapshot);
    promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
    assert!(!route.output_contract.requires_content_evidence);
    assert!(!route
        .route_reason
        .contains("active_clarify_locator_reply_execute"));
    assert!(route
        .route_reason
        .contains("preserve_active_clarify_output_contract"));
    assert!(route
        .route_reason
        .contains("active_clarify_structured_payload_execute"));
}

#[test]
fn clarify_structured_payload_reply_promotes_direct_route_to_execute() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "Inline payload can be answered directly".to_string(),
        needs_clarify: false,
        route_reason: "executionless_route_downgraded_to_direct_answer".to_string(),
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
                resolved_intent:
                    "Continue the previous request that was waiting for clarification: 把那个 JSON 数组按 score 排一下并转成表格\nUser now provides the missing target or content: [{\"name\":\"alpha\",\"score\":7},{\"name\":\"beta\",\"score\":12}]"
                        .to_string(),
                prior_user_text: "把那个 JSON 数组按 score 排一下并转成表格".to_string(),
                current_user_text: r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#
                    .to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::FollowupLocatorReply,
            },
        );

    preserve_active_clarify_output_contract_for_locator_reply(&mut route, &resolution, &snapshot);
    promote_active_clarify_structured_payload_reply_to_execute(&mut route, &resolution, &snapshot);
    promote_active_clarify_locator_reply_to_execute(&mut route, &resolution, &snapshot);

    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
    assert!(!route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(route
        .route_reason
        .contains("active_clarify_structured_payload_execute"));
    assert!(!route
        .route_reason
        .contains("active_clarify_locator_reply_execute"));
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

    repair_scalar_field_value_contract_for_locator_reply(
        &mut route,
        "读取 package.json 里的 name，只输出值",
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route
        .route_reason
        .contains("scalar_field_value_contract_repair"));
}

#[test]
fn scalar_field_value_contract_repair_overrides_locator_reply_existence_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Continue field extraction after the user supplied a file path"
            .to_string(),
        needs_clarify: false,
        route_reason:
            "structured_field_selector_requires_scalar_value; active_clarify_locator_reply_execute"
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
            response_shape: crate::OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::Path,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
            locator_hint: "package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };

    repair_scalar_field_value_contract_for_locator_reply(
        &mut route,
        "Continue the previous field extraction request with target package.json",
    );

    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route
        .route_reason
        .contains("scalar_field_value_contract_repair"));
}

#[test]
fn scalar_field_pair_contract_repair_uses_recent_scalar_equality_contract() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Extract two structured field values and compare them".to_string(),
        needs_clarify: false,
        route_reason:
            "llm_semantic_contract_repair:structured_field_selector_requires_scalar_value"
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
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "UI/package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };

    repair_scalar_field_value_contract_for_locator_reply(
        &mut route,
        "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name，最后只用一行输出：前者、后者、一样或不一样",
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RecentScalarEqualityCheck
    );
    assert!(route
        .route_reason
        .contains("scalar_field_pair_contract_repair"));
    assert!(!route
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
                    "Continue the previous request that was waiting for clarification: Send me the file\nUser now provides the missing target or content: README.md"
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
                    "Continue the previous request that was waiting for clarification: Send that config file\nUser now provides the missing target or content: /tmp/app_config.toml"
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

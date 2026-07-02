use super::super::{apply_ask_post_route, apply_post_route_refinements, route_reason_has_marker};
use super::{
    reject_direct_file_delivery_workspace_root_locator,
    unbound_existing_file_delivery_route_should_force_clarify,
};
use crate::{AgentRuntimeConfig, AppState, SkillViewsSnapshot};
use claw_core::config::{AgentConfig, ToolsConfig};
use std::collections::{HashMap, HashSet};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

fn make_temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rustclaw_file_delivery_{label}_{}_{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&path).expect("temp root");
    path
}

fn test_state_with_root(root: PathBuf) -> AppState {
    let agents_by_id = HashMap::from([(
        crate::DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            workspace_root: root.clone(),
            default_locator_search_dir: root,
            locator_scan_max_depth: 2,
            locator_scan_max_files: 100,
            tools_policy: Arc::new(
                crate::ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

fn executable_filename_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "deliver file".to_string(),
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

#[test]
fn active_anchor_file_delivery_requires_structured_reference() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Send /tmp/work/app_config.toml as an attachment".to_string();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/work/app_config.toml".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some("/tmp/work/app_config.toml".to_string()),
            ordered_entries: vec!["/tmp/work/app_config.toml".to_string()],
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        super::active_anchor_file_delivery_without_structured_reference_should_force_clarify(
            "send the config file",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn active_anchor_file_delivery_accepts_structured_reference() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Send /tmp/work/app_config.toml as an attachment".to_string();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/work/app_config.toml".to_string();
    let turn_analysis = turn_analysis_with_state_patch(serde_json::json!({
        "deictic_reference": {"target": "current_action_result"}
    }));
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some("/tmp/work/app_config.toml".to_string()),
            ordered_entries: vec!["/tmp/work/app_config.toml".to_string()],
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !super::active_anchor_file_delivery_without_structured_reference_should_force_clarify(
            "send the config file",
            &route,
            Some(&turn_analysis),
            &snapshot,
        )
    );
}

#[test]
fn active_anchor_file_delivery_accepts_ordered_entry_reference() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Send /tmp/work/model_io.log as an attachment".to_string();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/work/model_io.log".to_string();
    let turn_analysis = turn_analysis_with_state_patch(serde_json::json!({
        "ordered_entry_ref": {"relative_offset": 0},
        "active_ordered_entries_source": "recent_directory_listing",
        "active_ordered_entries_selector": {"target_kind": "file", "sort_by": "size_desc"}
    }));
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/work/logs".to_string()),
            ordered_entries: vec![
                "/tmp/work/app.log".to_string(),
                "/tmp/work/model_io.log".to_string(),
            ],
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !super::active_anchor_file_delivery_without_structured_reference_should_force_clarify(
            "send the selected file",
            &route,
            Some(&turn_analysis),
            &snapshot,
        )
    );
}

#[test]
fn active_anchor_file_delivery_accepts_reuse_active_turn_binding() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Send README.md as an attachment".to_string();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md".to_string();
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("README.md".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !super::active_anchor_file_delivery_without_structured_reference_should_force_clarify(
            "send this file",
            &route,
            Some(&turn_analysis),
            &snapshot,
        )
    );
}

#[test]
fn active_anchor_file_delivery_accepts_reuse_active_task_request_binding() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Send README.md as an attachment".to_string();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md".to_string();
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("README.md".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !super::active_anchor_file_delivery_without_structured_reference_should_force_clarify(
            "send this file",
            &route,
            Some(&turn_analysis),
            &snapshot,
        )
    );
}

#[test]
fn active_anchor_file_delivery_accepts_repaired_active_task_binding_marker() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Send README.md as an attachment".to_string();
    route.route_reason =
        "llm_semantic_contract_repair; contract_repair_marker=active_task_invalid_turn_binding"
            .to_string();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md".to_string();
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("README.md".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !super::active_anchor_file_delivery_without_structured_reference_should_force_clarify(
            "send this file",
            &route,
            Some(&turn_analysis),
            &snapshot,
        )
    );
}

#[test]
fn direct_file_delivery_directory_locator_defers_to_loop_before_deictic_guard() {
    let root = make_temp_root("delivery_dir_prebind");
    std::fs::create_dir_all(root.join("document")).expect("document dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent =
        "send the last file in the document directory, rejecting the previous file".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document directory".to_string();

    assert!(!reject_direct_file_delivery_workspace_root_locator(
        &state, "", &mut route
    ));

    assert_eq!(route.output_contract.locator_hint, "document directory");
    assert!(route.route_reason.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn direct_file_delivery_rejects_workspace_root_prebind_before_deictic_guard() {
    let root = make_temp_root("delivery_root_prebind_reject");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();

    assert!(reject_direct_file_delivery_workspace_root_locator(
        &state, "", &mut route
    ));

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(route.needs_clarify);
    assert_eq!(route.ask_mode, crate::AskMode::planner_execute_plain());
    assert!(route
        .route_reason
        .contains("direct_file_delivery_workspace_root_locator_rejected"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn generated_file_delivery_runtime_target_skips_workspace_root_prebind_reject() {
    let root = make_temp_root("generated_delivery_root_prebind_skip");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.wants_file_delivery = true;
    route.route_reason = "generated_file_delivery_allows_runtime_target".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();

    assert!(!reject_direct_file_delivery_workspace_root_locator(
        &state, "", &mut route
    ));

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route.needs_clarify);
    assert_eq!(route.ask_mode, crate::AskMode::planner_execute_plain());
    assert!(!route
        .route_reason
        .contains("direct_file_delivery_workspace_root_locator_rejected"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn explicit_missing_filename_delivery_contract_defers_not_found_to_execution() {
    let root = make_temp_root("delivery_explicit_missing_filename_contract");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "delivery-explicit-missing-filename-contract".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.wants_file_delivery = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint =
        "definitely_missing_named_file_rustclaw_001.txt".to_string();
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "把 definitely_missing_named_file_rustclaw_001.txt 发给我",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(
        !applied.execution_route_result.needs_clarify,
        "{}",
        applied.execution_route_result.route_reason
    );
    assert!(applied.execution_route_result.is_execute_gate());
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Filename
    );
    assert_eq!(
        applied.execution_route_result.output_contract.locator_hint,
        "definitely_missing_named_file_rustclaw_001.txt"
    );
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "inferred_missing_workspace_locator_requires_clarify"
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unresolved_file_delivery_current_request_filename_defers_to_loop_evidence() {
    let root = make_temp_root("delivery_missing_current_request_filename");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "delivery-missing-current-request-filename".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.needs_clarify = true;
    route.route_reason =
        "clarify_reason_code:missing_delivery_locator; unresolved_file_delivery_requires_clarify"
            .to_string();
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "把 definitely_missing_named_file_rustclaw_001.txt 发给我",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(applied.execution_route_result.needs_clarify);
    assert!(applied.execution_route_result.is_execute_gate());
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .is_empty());
    assert_eq!(
        applied.gate_record.reason_code,
        "post_route_file_delivery_current_request_locator_deferred_to_loop"
    );
    assert!(route_reason_has_marker(
        &applied.execution_route_result,
        "file_delivery_current_request_locator_deferred_to_loop"
    ));
    assert!(applied
        .prompt_with_memory_for_execution
        .contains("definitely_missing_named_file_rustclaw_001.txt"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unresolved_file_delivery_without_current_request_locator_stays_clarify() {
    let root = make_temp_root("delivery_missing_current_request_no_locator");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "delivery-missing-current-request-no-locator".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason =
        "clarify_reason_code:missing_delivery_locator; unresolved_file_delivery_requires_clarify"
            .to_string();
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "send it",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(applied.execution_route_result.needs_clarify);
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .is_empty());
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "file_delivery_current_request_locator_deferred_to_loop"
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unbound_existing_file_delivery_with_model_locator_forces_clarify() {
    let root = make_temp_root("unbound_delivery_model_locator");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.wants_file_delivery = true;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    assert!(unbound_existing_file_delivery_route_should_force_clarify(
        &state,
        "please send the referenced local configuration as a file",
        &route,
        false,
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unbound_existing_file_delivery_allows_current_request_locator() {
    let root = make_temp_root("delivery_current_locator");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.wants_file_delivery = true;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    assert!(!unbound_existing_file_delivery_route_should_force_clarify(
        &state,
        "please send configs/config.toml as a file",
        &route,
        false,
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unbound_existing_file_delivery_allows_authoritative_anchor() {
    let root = make_temp_root("delivery_authoritative_anchor");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.wants_file_delivery = true;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();

    assert!(!unbound_existing_file_delivery_route_should_force_clarify(
        &state,
        "please send it as a file",
        &route,
        true,
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unbound_existing_file_delivery_allows_generated_file_delivery() {
    let root = make_temp_root("delivery_generated_file");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.wants_file_delivery = true;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.route_reason = "generated_file_delivery".to_string();

    assert!(!unbound_existing_file_delivery_route_should_force_clarify(
        &state,
        "generate a small report and send it as a file",
        &route,
        false,
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unbound_existing_file_delivery_allows_resolved_workspace_child() {
    let root = make_temp_root("delivery_workspace_child");
    std::fs::create_dir_all(root.join("document")).expect("document dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.wants_file_delivery = true;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();

    assert!(!unbound_existing_file_delivery_route_should_force_clarify(
        &state,
        "please send document as a file",
        &route,
        false,
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_file_delivery_without_structured_selection_requires_boundary_clarify() {
    let root = make_temp_root("directory_delivery_requires_selection");
    let device_dir = root.join("device_local");
    std::fs::create_dir_all(&device_dir).expect("device dir");
    std::fs::write(device_dir.join("package.json"), "{}\n").expect("package fixture");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "directory-delivery-requires-selection".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.route_reason = "generated_file_delivery_allows_runtime_target".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "device_local".to_string();
    route.output_contract.requires_content_evidence = true;
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "send device_local as a file",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(
        applied.execution_route_result.needs_clarify,
        "{}",
        applied.execution_route_result.route_reason
    );
    assert_eq!(
        applied.execution_route_result.gate_kind(),
        crate::RouteGateKind::Execute
    );
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "directory_file_delivery_requires_structured_selection"
    ));
    assert!(route_reason_has_marker(
        &applied.execution_route_result,
        "unresolved_file_delivery_requires_clarify"
    ));
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "clarify_reason_code:missing_delivery_locator"
    ));
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_directory_file_delivery_marker_defers_to_agent_loop() {
    let root = make_temp_root("directory_delivery_marker_defer");
    let device_dir = root.join("device_local");
    std::fs::create_dir_all(&device_dir).expect("device dir");
    std::fs::write(device_dir.join("package.json"), "{}\n").expect("package fixture");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "directory-delivery-marker-defer".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.needs_clarify = true;
    route.clarify_question = "select a file".to_string();
    route.route_reason = "directory_file_delivery_requires_structured_selection".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "device_local".to_string();
    route.output_contract.requires_content_evidence = true;
    let mut post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: Some(device_dir.display().to_string()),
        auto_locator_hint: None,
        auto_locator_resolved_direct: true,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::default(),
    };
    let session_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut pre_loop_clarify_candidates = Vec::new();

    apply_post_route_refinements(
        &state,
        &task,
        "send ./device_local as a file",
        None,
        &session_snapshot,
        &mut pre_loop_clarify_candidates,
        &mut post_route,
    );

    assert!(!post_route.execution_route_result.needs_clarify);
    assert!(post_route
        .execution_route_result
        .clarify_question
        .is_empty());
    assert_eq!(
        post_route.gate_record.reason_code,
        "post_route_directory_file_delivery_deferred_to_agent_loop"
    );
    assert_eq!(
        post_route.gate_record.outcome,
        crate::post_route_policy::PostRoutePolicyOutcome::RefineContract
    );
    assert_eq!(
        pre_loop_clarify_candidates.as_slice(),
        ["directory_file_delivery_requires_structured_selection"]
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_file_delivery_with_structured_file_selector_stays_executable() {
    let root = make_temp_root("directory_delivery_selector_executable");
    let device_dir = root.join("device_local");
    std::fs::create_dir_all(&device_dir).expect("device dir");
    std::fs::write(device_dir.join("alpha.txt"), "alpha\n").expect("alpha fixture");
    std::fs::write(device_dir.join("beta.txt"), "beta\n").expect("beta fixture");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "directory-delivery-selector-executable".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.route_reason = "normalizer_emitted_directory_file_selector".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "device_local".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.self_extension.list_selector = crate::OutputListSelector {
        target_kind: crate::OutputScalarCountTargetKind::File,
        target_kind_specified: true,
        limit: Some(1),
        sort_by: Some("name_desc".to_string()),
        include_metadata: Some(false),
        include_hidden: Some(false),
    };
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "send the selected file from device_local",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(
        !applied.execution_route_result.needs_clarify,
        "{}",
        applied.execution_route_result.route_reason
    );
    assert!(applied.execution_route_result.is_execute_gate());
    assert!(applied.execution_route_result.wants_file_delivery);
    assert!(
        applied
            .execution_route_result
            .output_contract
            .delivery_required
    );
    assert_eq!(
        applied
            .execution_route_result
            .output_contract
            .self_extension
            .list_selector
            .sort_by
            .as_deref(),
        Some("name_desc")
    );
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "directory_file_delivery_requires_structured_selection"
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_rebinds_clarified_file_delivery_to_active_read_target_after_guards() {
    let root = make_temp_root("post_route_delivery_active_read");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Fixture\n").expect("readme fixture");
    let target = readme.display().to_string();
    let state = test_state_with_root(root.clone()).with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "post-route-delivery-active-read".to_string(),
        user_id: 7,
        chat_id: 8,
        user_key: Some("post-route-delivery-active-read-user".to_string()),
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let frame = crate::followup_frame::FollowupFrame {
        source_request: "read README".to_string(),
        op_kind: crate::followup_frame::FollowupOpKind::Read,
        bound_target: Some(target.clone()),
        source_task_id: "previous-read-task".to_string(),
        updated_at_ts: 1,
        expires_at_ts: u64::MAX,
        ..crate::followup_frame::FollowupFrame::default()
    };
    let frame_json = serde_json::to_string(&frame).expect("serialize followup frame");
    {
        let db = state.core.db.get().expect("db");
        db.execute(
            "INSERT INTO followup_frames (
                user_id, chat_id, user_key, frame_json, source_task_id, updated_at_ts, expires_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
                frame_json = excluded.frame_json,
                source_task_id = excluded.source_task_id,
                updated_at_ts = excluded.updated_at_ts,
                expires_at_ts = excluded.expires_at_ts",
            rusqlite::params![
                task.user_id,
                task.chat_id,
                task.user_key.as_deref().unwrap(),
                frame_json,
                frame.source_task_id,
                frame.updated_at_ts as i64,
                frame.expires_at_ts as i64,
            ],
        )
        .expect("persist followup frame");
    }

    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.resolved_intent =
        "deliver active bound target from the latest structured read frame".to_string();
    route.route_reason = concat!(
        "clarify_reason_code:missing_delivery_locator; ",
        "active_anchor_file_delivery_requires_structured_reference; ",
        "background_locator_requires_clarify"
    )
    .to_string();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "deliver that active file",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(
        !applied.execution_route_result.needs_clarify,
        "{}",
        applied.execution_route_result.route_reason
    );
    assert!(applied.execution_route_result.is_execute_gate());
    assert!(applied.execution_route_result.wants_file_delivery);
    assert!(
        applied
            .execution_route_result
            .output_contract
            .delivery_required
    );
    assert_eq!(
        applied.execution_route_result.output_contract.locator_hint,
        target
    );
    assert_eq!(applied.auto_locator_path.as_deref(), Some(target.as_str()));
    assert!(route_reason_has_marker(
        &applied.execution_route_result,
        "structural_file_delivery_bound_to_recent_read_target"
    ));
    let _ = std::fs::remove_dir_all(root);
}

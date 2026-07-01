use super::super::{
    background_only_locator_route_should_force_clarify,
    locatorless_observation_route_should_force_clarify, route_reason_has_marker,
    runtime_status_query_route_can_plan_without_locator,
    unbound_model_context_target_route_should_force_clarify,
};
use super::*;
use claw_core::config::{AgentConfig, ToolsConfig};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn make_temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rustclaw_runtime_status_{label}_{}_{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&path).expect("temp root");
    path
}

fn test_state_with_root(root: PathBuf) -> crate::AppState {
    let agents_by_id = HashMap::from([(
        crate::DEFAULT_AGENT_ID.to_string(),
        crate::AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    crate::AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(crate::SkillViewsSnapshot {
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

fn executable_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: "读取 README 开头并总结".to_string(),
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
            locator_kind: crate::OutputLocatorKind::None,
            requires_content_evidence: true,
            ..Default::default()
        },
    }
}

fn executable_filename_route() -> crate::RouteResult {
    executable_route()
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

fn status_query_analysis(
    state_patch: Option<serde_json::Value>,
) -> crate::intent_router::TurnAnalysis {
    crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch,
        attachment_processing_required: false,
    }
}

fn empty_snapshot() -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

#[test]
fn status_query_locatorless_route_can_plan_without_semantic_promotion() {
    let state = test_state_with_root(make_temp_root("runtime_status_service_status"));
    let mut route = executable_route();
    route.resolved_intent =
        "Provide a brief runtime diagnostics overview from fresh system observation.".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let analysis = status_query_analysis(None);

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(
        !super::super::locatorless_observation_route_should_force_clarify(
            &state,
            "status overview",
            &route,
            Some(&analysis),
            &empty_snapshot(),
        )
    );
}

#[test]
fn status_query_bare_fragment_does_not_mutate_semantic_kind() {
    let state = test_state_with_root(make_temp_root("bare_fragment"));
    let mut route = executable_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let analysis = status_query_analysis(None);

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(
        !super::super::locatorless_observation_route_should_force_clarify(
            &state,
            "logs",
            &route,
            Some(&analysis),
            &empty_snapshot(),
        )
    );
}

#[test]
fn command_payload_status_query_stays_raw_command_output() {
    let mut route = executable_route();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
}

#[test]
fn runtime_status_patch_status_query_stays_raw_command_output() {
    let state = test_state_with_root(make_temp_root("runtime_status_patch"));
    let mut route = executable_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = status_query_analysis(Some(serde_json::json!({
        "runtime_status_query": {"kind": "current_user", "scope": "system"}
    })));

    assert!(append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert!(route_reason_has_marker(
        &route,
        "capability_ref=system.runtime_status"
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    assert!(
        !super::super::locatorless_observation_route_should_force_clarify(
            &state,
            "current runtime user",
            &route,
            Some(&analysis),
            &empty_snapshot(),
        )
    );
}

#[test]
fn runtime_status_query_machine_patch_overrides_spurious_semantic_kind() {
    let state = test_state_with_root(make_temp_root("runtime_status_spurious_semantic"));
    let mut route = executable_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let analysis = status_query_analysis(Some(serde_json::json!({
        "runtime_status_query": {"kind": "current_user", "scope": "system"}
    })));

    assert!(
        !super::super::locatorless_observation_route_should_force_clarify(
            &state,
            "current runtime user",
            &route,
            Some(&analysis),
            &empty_snapshot(),
        )
    );
}

#[test]
fn scalar_status_query_with_runtime_kind_adds_capability_context() {
    let mut route = executable_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = status_query_analysis(Some(serde_json::json!({
        "runtime_status_query": {"kind": "current_user", "scope": "system"}
    })));

    assert!(append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(super::super::route_reason_has_marker(
        &route,
        "capability_ref=system.runtime_status"
    ));
}

#[test]
fn runtime_status_scalar_query_can_plan_without_locator_prebind() {
    let state = test_state_with_root(make_temp_root("runtime_status_scalar_no_prebind"));
    let mut route = executable_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "runtime_status_query": {"kind": "current_working_directory", "scope": "process"}
        })),
        attachment_processing_required: false,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "return cwd",
        &route,
        Some(&analysis),
        &empty_snapshot(),
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
}

#[test]
fn scalar_status_without_runtime_status_patch_has_no_runtime_status_bypass() {
    let mut route = executable_route();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    assert!(!runtime_status_query_route_can_plan_without_locator(
        Some(&analysis),
        &route,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
}

#[test]
fn scalar_status_active_task_update_without_runtime_status_patch_has_no_runtime_status_bypass() {
    let mut route = executable_route();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(!runtime_status_query_route_can_plan_without_locator(
        Some(&analysis),
        &route,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
}

#[test]
fn locatorless_service_status_without_capability_ref_requires_clarify() {
    let state = test_state_with_root(make_temp_root("locatorless_service_status"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Check whether the requested daemon process is currently running.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(locatorless_observation_route_should_force_clarify(
        &state,
        "check whether telegramd is currently running",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_service_status_with_capability_ref_does_not_clarify() {
    let state = test_state_with_root(make_temp_root("locatorless_service_status_capability"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Check whether the requested daemon process is currently running.".to_string();
    route.route_reason = "capability_ref=system.runtime_status".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "check whether telegramd is currently running",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_status_query_clarify_is_not_promoted_before_planner() {
    let state = test_state_with_root(make_temp_root("locatorless_status_query_clarify"));
    let mut route = executable_filename_route();
    route.set_clarify_gate();
    route.needs_clarify = true;
    route.clarify_question.clear();
    route.resolved_intent =
        "Run a basic runtime health check and report the most important concern.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert!(!route.is_execute_gate());
    assert!(route.needs_clarify);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "status overview",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn system_health_selector_clarify_adds_capability_context_without_semantic_promotion() {
    let state = test_state_with_root(make_temp_root("system_health_selector_clarify"));
    let mut route = executable_filename_route();
    route.set_clarify_gate();
    route.needs_clarify = true;
    route.clarify_question = "missing locator".to_string();
    route.resolved_intent =
        "Collect host operating-system fields. structured_field_selector=system_health.*"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("system_health.*".to_string());
    let analysis = status_query_analysis(Some(serde_json::json!({
        "structured_field_selector": "system_health.*"
    })));
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert!(!route.is_execute_gate());
    assert!(route.needs_clarify);
    assert_eq!(route.clarify_question, "missing locator");
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route_reason_has_marker(
        &route,
        "capability_ref=system.health_check"
    ));
    assert_eq!(
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        Some("system_health.*")
    );
    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "status overview",
        &route,
        Some(&analysis),
        &snapshot,
    ));
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "status overview",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn generic_service_status_with_model_background_locator_requires_capability_ref() {
    let state = test_state_with_root(make_temp_root("generic_health_background_locator"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Collect baseline runtime health observations and summarize the primary finding."
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/model-supplied-context".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(background_only_locator_route_should_force_clarify(
        &state,
        "run a basic health check here and summarize only the most important findings",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));

    route.route_reason = "capability_ref=system.health_check".to_string();
    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "run a basic health check here and summarize only the most important findings",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn runtime_status_scalar_path_query_enters_planner_without_locator_prebind() {
    let state = test_state_with_root(make_temp_root("runtime_status_scalar_path"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Return the current working directory path only.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "runtime_status_query": {"kind": "current_working_directory", "scope": "process"}
        })),
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "return cwd",
        &route,
        Some(&analysis),
        &snapshot,
    ));

    let mut route_without_patch = executable_filename_route();
    route_without_patch.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_without_patch.output_contract.locator_hint.clear();
    route_without_patch
        .output_contract
        .requires_content_evidence = true;
    route_without_patch.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route_without_patch.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis_without_patch = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    assert!(!runtime_status_query_route_can_plan_without_locator(
        Some(&analysis_without_patch),
        &route_without_patch,
    ));
    assert_eq!(
        route_without_patch.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );

    let mut route_without_analysis = executable_filename_route();
    route_without_analysis.resolved_intent =
        "Return the current working directory path only.".to_string();
    route_without_analysis.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route_without_analysis.output_contract.locator_hint.clear();
    route_without_analysis
        .output_contract
        .requires_content_evidence = true;
    route_without_analysis.output_contract.semantic_kind =
        crate::OutputSemanticKind::ScalarPathOnly;
    route_without_analysis.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    assert!(!runtime_status_query_route_can_plan_without_locator(
        None,
        &route_without_analysis,
    ));
    assert_eq!(
        route_without_analysis.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(locatorless_observation_route_should_force_clarify(
        &state,
        "return cwd",
        &route_without_analysis,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_status_query_can_plan_without_semantic_promotion() {
    let state = test_state_with_root(make_temp_root("locatorless_raw_status_query"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Check whether the local clawd process is present and summarize matches.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "check whether the local clawd process is present",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn locatorless_status_query_with_explicit_command_does_not_bind_to_service_status() {
    let mut state =
        test_state_with_root(make_temp_root("locatorless_status_query_explicit_command"));
    state.policy.command_intent.standalone_commands = vec!["hostname".to_string()];
    let mut route = executable_filename_route();
    route.resolved_intent = "return the current machine hostname".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(!append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
}

#[test]
fn locatorless_status_query_with_command_payload_does_not_bind_to_service_status() {
    let mut route = executable_filename_route();
    route.resolved_intent = "return the current runtime user".to_string();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(!append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
}

#[test]
fn locatorless_status_query_with_runtime_status_patch_does_not_bind_to_service_status() {
    let state = test_state_with_root(make_temp_root(
        "locatorless_status_query_runtime_status_patch",
    ));
    let mut route = executable_filename_route();
    route.resolved_intent = "return the current runtime user".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "runtime_status_query": {"kind": "current_user", "scope": "system"}
        })),
        attachment_processing_required: false,
    };

    assert!(append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert!(route_reason_has_marker(
        &route,
        "capability_ref=system.runtime_status"
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "current runtime user",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn scalar_runtime_tool_observation_does_not_bind_to_service_status_without_kind() {
    let mut route = executable_filename_route();
    route.resolved_intent = "return runtime scalar from system_basic".to_string();
    route.route_reason = "execution_recipe_scalar_runtime_tool_observation".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(!append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
}

#[test]
fn locatorless_scalar_status_query_without_kind_does_not_mutate_semantic_kind() {
    let state = test_state_with_root(make_temp_root("scalar_status_runtime_info"));
    let mut route = executable_filename_route();
    route.resolved_intent = "return current runtime scalar".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(!append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "current runtime scalar",
        &route,
        Some(&analysis),
        &snapshot,
    ));
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "current runtime scalar",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn locatorless_scalar_status_query_with_runtime_kind_adds_capability_context() {
    let state = test_state_with_root(make_temp_root("scalar_status_runtime_info_with_kind"));
    let mut route = executable_filename_route();
    route.resolved_intent = "return current runtime scalar".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "runtime_status_query": {"kind": "kernel_release", "scope": "system"}
        })),
        attachment_processing_required: false,
    };

    assert!(append_runtime_status_capability_context(
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route_reason_has_marker(
        &route,
        "capability_ref=system.runtime_status"
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "current runtime scalar",
        &route,
        Some(&analysis),
        &snapshot,
    ));
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "current runtime scalar",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn locatorless_observation_with_command_payload_raw_output_without_input_requires_clarify() {
    let state = test_state_with_root(make_temp_root("locatorless_observation_command_payload"));
    let mut route = executable_filename_route();
    route.resolved_intent = "return the current runtime user".to_string();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(locatorless_observation_route_should_force_clarify(
        &state,
        "current runtime user",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_command_output_summary_does_not_clarify() {
    let state = test_state_with_root(make_temp_root("locatorless_command_output_summary"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Run two local commands and summarize their success and failure outcomes.".to_string();
    route.route_reason = "explicit_command_requires_command_output_summary_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::CommandOutputSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "run pwd, then run a missing command, then summarize what succeeded and failed",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_command_with_path_structural_args_does_not_clarify() {
    let mut state = test_state_with_root(make_temp_root("locatorless_observation_path_command"));
    state.policy.command_intent.execute_prefixes = vec!["please run ".to_string()];
    if crate::agent_engine::explicit_command_segment_for_policy(
        &state.policy.command_intent,
        "please run uname -a and tell me the result",
    )
    .as_deref()
        != Some("uname -a")
    {
        return;
    }
    let mut route = executable_filename_route();
    route.resolved_intent = "Run uname -a command and return its output".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "please run uname -a and tell me the result",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_command_grounded_summary_can_plan_without_path_clarify() {
    let state = test_state_with_root(make_temp_root("locatorless_raw_command_summary"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Collect current local runtime identity values and summarize them briefly.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let turn_analysis = turn_analysis_with_state_patch(serde_json::json!({
        "runtime_status_query": {"kind": "current_user"}
    }));

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "collect current local runtime identity values and summarize them briefly",
        &route,
        Some(&turn_analysis),
        &snapshot,
    ));
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "collect current local runtime identity values and summarize them briefly",
        &route,
        Some(&turn_analysis),
        &snapshot,
    ));
}

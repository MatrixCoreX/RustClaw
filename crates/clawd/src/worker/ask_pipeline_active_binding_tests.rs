use super::{
    prebind_active_bound_target_for_locatorless_content_evidence,
    prebind_active_bound_target_from_matching_locator_hint,
    prebind_active_listing_target_for_locatorless_scalar_count,
    prebind_session_alias_locator_from_current_request,
    repair_service_status_file_locator_to_content_excerpt,
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
        "rustclaw_active_binding_{label}_{}_{}",
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
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
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

#[test]
fn service_status_with_file_locator_repairs_to_content_evidence() {
    let root = make_temp_root("service_status_file_locator");
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).expect("logs");
    let target = logs.join("act_plan.log");
    std::fs::write(&target, "phase=loop_done\n").expect("log");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/act_plan.log".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;

    assert!(repair_service_status_file_locator_to_content_excerpt(
        &state, &mut route,
    ));

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        route.output_contract.locator_hint,
        target
            .canonicalize()
            .expect("canonical")
            .display()
            .to_string()
    );
    assert!(route
        .route_reason
        .contains("service_status_file_locator_repaired_to_content_excerpt"));
}

#[test]
fn active_bound_target_prebinds_matching_basename_locator_hint() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "test_bundle.zip".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveList;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some(
                "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
            ),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(prebind_active_bound_target_from_matching_locator_hint(
        &mut route, &snapshot,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
    );
    assert!(route
        .route_reason
        .contains("active_bound_target_prebound_from_matching_locator_hint"));
}

#[test]
fn active_bound_target_prebinds_locatorless_content_summary() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/work/README.md".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(prebind_active_bound_target_for_locatorless_content_evidence(&mut route, &snapshot,));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(route.output_contract.locator_hint, "/tmp/work/README.md");
    assert!(
        !super::super::unbound_targeted_evidence_route_should_force_clarify(
            "Summarize the current result in one sentence.",
            &route,
            &snapshot,
            "<none>",
        )
    );
    assert!(route
        .route_reason
        .contains("active_bound_target_prebound_for_locatorless_content_evidence"));
}

#[test]
fn active_listing_target_prebinds_locatorless_scalar_count_clarify() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/work/docs".to_string()),
            ordered_entries: vec![
                "release_checklist.md".to_string(),
                "service_notes.md".to_string(),
            ],
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some("/tmp/work/docs".to_string()),
            ordered_entries: vec![
                "release_checklist.md".to_string(),
                "service_notes.md".to_string(),
            ],
            observed_entry_count: Some(2),
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };

    assert!(prebind_active_listing_target_for_locatorless_scalar_count(
        &mut route, &snapshot,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(route.output_contract.locator_hint, "/tmp/work/docs");
    assert!(route
        .route_reason
        .contains("active_listing_target_prebound_for_locatorless_scalar_count"));
}

#[test]
fn session_alias_locator_overrides_workspace_basename_locator() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/docs".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "that docs dir".to_string(),
                target: "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs"
                    .to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(prebind_session_alias_locator_from_current_request(
        "look at that docs dir, names only",
        &mut route,
        &snapshot,
    ));
    assert_eq!(
        route.output_contract.locator_hint,
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs"
    );
    assert!(route
        .route_reason
        .contains("session_alias_locator_prebound_from_current_request"));
}

#[test]
fn session_alias_locator_prebinds_raw_command_output_when_evidence_required() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "that log".to_string(),
                target: "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/app.log"
                    .to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(prebind_session_alias_locator_from_current_request(
        "show that log",
        &mut route,
        &snapshot,
    ));
    assert_eq!(
        route.output_contract.locator_hint,
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/app.log"
    );
    assert!(route
        .route_reason
        .contains("session_alias_locator_prebound_from_current_request"));
}

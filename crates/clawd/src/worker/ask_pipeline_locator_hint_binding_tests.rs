use super::super::{locatorless_observation_route_should_force_clarify, route_reason_has_marker};
use super::prebind_workspace_root_locator_from_resolved_prompt;
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
        "rustclaw_locator_hint_binding_{label}_{}_{}",
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
fn locatorless_observation_binds_workspace_root_from_resolved_prompt_path() {
    let root = make_temp_root("locatorless_workspace_root");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.resolved_intent = format!(
        "List the first 10 entry names in the repository root {} without explanation.",
        root.display()
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(prebind_workspace_root_locator_from_resolved_prompt(
        &state,
        &route.resolved_intent.clone(),
        &mut route,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        route.output_contract.locator_hint,
        root.display().to_string()
    );
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "列出当前仓库根目录前 10 个条目名称，不要解释",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn clarify_scalar_count_binds_workspace_root_from_resolved_prompt_path() {
    let root = make_temp_root("clarify_scalar_count_workspace_root");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.resolved_intent = format!(
        "Count direct child directories in {} and exclude hidden entries.",
        root.display()
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(prebind_workspace_root_locator_from_resolved_prompt(
        &state,
        &route.resolved_intent.clone(),
        &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        route.output_contract.locator_hint,
        root.display().to_string()
    );
    assert!(route_reason_has_marker(
        &route,
        "workspace_root_locator_prebound_from_resolved_prompt"
    ));
}

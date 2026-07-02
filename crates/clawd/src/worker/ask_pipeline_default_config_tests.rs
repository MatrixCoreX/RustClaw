use super::super::{agent_loop_default_context, build_loop_context_after_boundary_preflight};
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
        "rustclaw_default_config_{label}_{}_{}",
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

fn test_task(task_id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn config_route(route_marker: &str, resolved_intent: &str) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_with_chat_finalizer(),
        resolved_intent: resolved_intent.to_string(),
        needs_clarify: false,
        route_reason: route_marker.to_string(),
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
            locator_kind: crate::OutputLocatorKind::Path,
            requires_content_evidence: true,
            response_shape: crate::OutputResponseShape::Free,
            ..Default::default()
        },
    }
}

#[test]
fn config_contract_default_main_config_survives_product_name_auto_locator() {
    let root = make_temp_root("contract_survives_product_name");
    std::fs::create_dir_all(root.join("configs")).expect("create configs dir");
    std::fs::write(
        root.join("configs/config.toml"),
        "selected_vendor = \"minimax\"\n",
    )
    .expect("write main config");
    std::fs::write(root.join("rustclaw"), "not the main config\n").expect("write product child");
    let state = test_state_with_root(root.clone());
    let task = test_task("config-contract-default-main-config");
    let route = config_route(
        "config_validation",
        "Audit the product main config without exposing secret values.",
    );
    let resolved_intent = route.resolved_intent.clone();

    let applied = build_loop_context_after_boundary_preflight(
        &state,
        &task,
        "检查 RustClaw 主配置有没有明显风险，不能泄露任何密钥值",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(!applied.execution_route_result.needs_clarify);
    assert!(applied.execution_route_result.is_execute_gate());
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(applied.auto_locator_path.is_none());
    assert!(!applied
        .prompt_with_memory_for_execution
        .contains(&root.join("rustclaw").display().to_string()));
    assert!(applied
        .resolved_prompt_for_execution
        .contains("default_main_config_contract"));
    assert!(applied
        .resolved_prompt_for_execution
        .contains("configs/config.toml"));
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("config_contract_default_main_config_deferred_to_loop"));
    assert!(!applied
        .execution_route_result
        .route_reason
        .contains("_prebound"));
    let loop_ctx = agent_loop_default_context(Some(crate::agent_engine::AgentRunContext {
        route_result: Some(applied.execution_route_result.clone()),
        ..Default::default()
    }))
    .expect("loop context");
    let route = loop_ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn config_risk_default_main_config_replaces_workspace_identity_locator_hint() {
    let root = make_temp_root("rustclaw");
    std::fs::create_dir_all(root.join("configs")).expect("create configs dir");
    std::fs::write(
        root.join("configs/config.toml"),
        "selected_vendor = \"minimax\"\n",
    )
    .expect("write main config");
    let state = test_state_with_root(root.clone());
    let task = test_task("config-risk-default-main-config-workspace-identity");
    let mut route = config_route(
        "config_risk_assessment",
        "Assess the workspace main configuration and redact sensitive values.",
    );
    route.set_clarify_gate();
    route.needs_clarify = true;
    route
        .route_reason
        .push_str("; semantic_contract_requires_evidence; clarify_reason_code:missing_read_target");
    let workspace_identity_hint = format!(
        "{}.toml",
        root.file_name()
            .and_then(|name| name.to_str())
            .expect("workspace basename")
    );
    route.output_contract.locator_hint = workspace_identity_hint;
    let resolved_intent = route.resolved_intent.clone();

    let applied = build_loop_context_after_boundary_preflight(
        &state,
        &task,
        "audit the workspace main configuration without exposing secret values",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(applied.execution_route_result.needs_clarify);
    assert_eq!(
        applied.execution_route_result.output_contract.locator_hint,
        ""
    );
    assert!(applied.auto_locator_path.is_none());
    assert!(applied
        .resolved_prompt_for_execution
        .contains("default_main_config_contract"));
    assert!(applied
        .resolved_prompt_for_execution
        .contains("configs/config.toml"));
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("config_contract_default_main_config_deferred_to_loop"));
    assert!(!applied
        .execution_route_result
        .route_reason
        .contains("_prebound"));
    let _ = std::fs::remove_dir_all(root);
}

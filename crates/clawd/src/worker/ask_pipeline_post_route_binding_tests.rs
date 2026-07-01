use super::auto_locator_scalar_file_without_current_locator_should_force_clarify;
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
        "rustclaw_post_route_binding_{label}_{}_{}",
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
        output_contract: crate::IntentOutputContract::default(),
    }
}

#[test]
fn auto_locator_scalar_file_without_current_locator_requires_clarify() {
    let root = make_temp_root("auto_locator_scalar_file");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let config_path = config_dir.join("app_config.toml");
    std::fs::write(&config_path, "[app]\nname = \"Demo\"\n").expect("config");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("app.name".to_string());

    assert!(
        auto_locator_scalar_file_without_current_locator_should_force_clarify(
            &state,
            "read app.name from that config and output only the value",
            &route,
            Some(config_path.to_str().expect("utf8 path")),
        )
    );
}

#[test]
fn session_alias_prebound_scalar_file_allows_auto_locator_guard() {
    let root = make_temp_root("auto_locator_session_alias");
    let docs_dir = root.join("docs");
    std::fs::create_dir_all(&docs_dir).expect("docs dir");
    let target_path = docs_dir.join("service_notes.md");
    std::fs::write(&target_path, "# Service Notes\n").expect("service notes");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = target_path.display().to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("heading".to_string());
    route.route_reason = "session_alias_locator_prebound_from_current_request".to_string();

    assert!(
        !auto_locator_scalar_file_without_current_locator_should_force_clarify(
            &state,
            "read the heading from alias file and output only the heading",
            &route,
            Some(target_path.to_str().expect("utf8 path")),
        )
    );
}

#[test]
fn explicit_file_scalar_route_allows_auto_locator_field_route() {
    let root = make_temp_root("auto_locator_field_selector_explicit");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let config_path = config_dir.join("app_config.toml");
    std::fs::write(&config_path, "[app]\nname = \"Demo\"\n").expect("config");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/app_config.toml".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("app.name".to_string());

    assert!(
        !auto_locator_scalar_file_without_current_locator_should_force_clarify(
            &state,
            "read configs/app_config.toml app.name and output only the value",
            &route,
            Some(config_path.to_str().expect("utf8 path")),
        )
    );
}

#[test]
fn scalar_file_without_structured_field_contract_does_not_force_clarify() {
    let root = make_temp_root("auto_locator_scalar_no_field_contract");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    let config_path = config_dir.join("app_config.toml");
    std::fs::write(&config_path, "[app]\nname = \"Demo\"\n").expect("config");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(
        !auto_locator_scalar_file_without_current_locator_should_force_clarify(
            &state,
            "read the scalar value from that config",
            &route,
            Some(config_path.to_str().expect("utf8 path")),
        )
    );
}

use super::{contract_repair_candidate_observations, registry_capability_contract_observation};
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
        "rustclaw_contract_repair_{label}_{}_{}",
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

#[test]
fn registry_capability_observation_records_locatorless_contract_conflict() {
    let mut route = executable_filename_route();
    route.resolved_intent =
        "List knowledge base namespaces capability_ref=kb.list_namespaces field=names field=count"
            .to_string();
    route.route_reason =
        "llm_semantic_contract_repair changed bounded listing to directory_entry_groups"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;

    let observation =
        registry_capability_contract_observation("", &route).expect("capability observation");

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryEntryGroups
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        observation
            .get("capability_refs")
            .and_then(serde_json::Value::as_array)
            .and_then(|items| items.first())
            .and_then(serde_json::Value::as_str),
        Some("kb.list_namespaces")
    );
    assert_eq!(
        observation
            .get("has_conflicting_route_contract")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(observation.get("legacy_semantic_kind").is_none());
}

#[test]
fn registry_capability_observation_keeps_spurious_delivery_as_evidence() {
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Search KB namespace capability_ref=kb.search namespace=docs query=service_status top_k=3"
            .to_string();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "docs".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;

    let observation =
        registry_capability_contract_observation("", &route).expect("capability observation");

    assert!(route.needs_clarify);
    assert_eq!(route.gate_kind(), crate::RouteGateKind::Clarify);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(route.output_contract.locator_hint, "docs");
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        observation
            .get("capability_refs")
            .and_then(serde_json::Value::as_array)
            .and_then(|items| items.first())
            .and_then(serde_json::Value::as_str),
        Some("kb.search")
    );
    assert_eq!(
        observation
            .get("delivery_required")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        observation
            .get("needs_clarify")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
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
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            locator_kind: crate::OutputLocatorKind::Filename,
            locator_hint: "README.md".to_string(),
            requires_content_evidence: true,
            ..Default::default()
        },
    }
}

fn contract_candidate<'a>(
    candidates: &'a [serde_json::Value],
    source: &str,
) -> &'a serde_json::Value {
    candidates
        .iter()
        .find(|candidate| {
            candidate.get("source").and_then(serde_json::Value::as_str) == Some(source)
        })
        .unwrap_or_else(|| panic!("missing contract candidate source {source}: {candidates:?}"))
}

fn assert_candidate(
    candidate: &serde_json::Value,
    contract_ref: &str,
    locator_tail: Option<&str>,
    response_shape: Option<&str>,
) {
    assert_eq!(
        candidate
            .get("contract_ref")
            .and_then(serde_json::Value::as_str),
        Some(contract_ref)
    );
    if let Some(locator_tail) = locator_tail {
        assert!(
            candidate
                .get("locator_hint")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|locator| locator.ends_with(locator_tail)),
            "{candidate:?}"
        );
    }
    if let Some(response_shape) = response_shape {
        assert_eq!(
            candidate
                .get("response_shape")
                .and_then(serde_json::Value::as_str),
            Some(response_shape)
        );
    }
}

#[test]
fn command_observation_marker_repair_preserves_command_summary_contract() {
    let mut route = executable_filename_route();
    route.route_reason =
        "explicit_command_requires_command_output_summary_execution; command_result_synthesis"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/run".to_string();
    route.wants_file_delivery = false;

    let state = test_state_with_root(make_temp_root("command_observation_candidate"));
    let candidates = contract_repair_candidate_observations(&state, "", "", &route);
    let candidate = contract_candidate(&candidates, "command_observation_marker");

    assert_candidate(candidate, "contract:command_output_summary", None, None);
}

#[test]
fn sqlite_path_excerpt_judgment_contract_repair_uses_db_kind() {
    let root = make_temp_root("sqlite_path_excerpt_judgment_repair");
    std::fs::create_dir_all(root.join("data")).expect("data directory");
    std::fs::write(root.join("data/db-basic-contract.sqlite"), "").expect("sqlite file");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.route_reason = "excerpt_kind_judgment".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "data/db-basic-contract.sqlite".to_string();

    let candidates = contract_repair_candidate_observations(
        &state,
        "inspect data/db-basic-contract.sqlite",
        "",
        &route,
    );
    let candidate = contract_candidate(&candidates, "sqlite_path_excerpt_judgment");
    assert_candidate(
        candidate,
        "contract:sqlite_database_kind_judgment",
        Some("data/db-basic-contract.sqlite"),
        None,
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn sqlite_structured_version_contract_repair_uses_schema_version_contract() {
    let root = make_temp_root("sqlite_structured_version_repair");
    std::fs::create_dir_all(root.join("data")).expect("data directory");
    std::fs::write(root.join("data/skill-calls-smoke.sqlite"), "").expect("sqlite file");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "data/skill-calls-smoke.sqlite".to_string();
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("sqlite.user_version".to_string());

    let candidates = contract_repair_candidate_observations(
        &state,
        "inspect data/skill-calls-smoke.sqlite",
        "",
        &route,
    );
    let candidate = contract_candidate(&candidates, "sqlite_structured_version");
    assert_candidate(
        candidate,
        "contract:sqlite_schema_version",
        Some("data/skill-calls-smoke.sqlite"),
        Some("scalar"),
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn sqlite_structured_version_selector_overrides_spurious_semantic_kind() {
    let root = make_temp_root("sqlite_structured_version_spurious_semantic");
    std::fs::create_dir_all(root.join("data")).expect("data directory");
    std::fs::write(root.join("data/app.sqlite"), "").expect("sqlite file");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "data/app.sqlite".to_string();
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("sqlite.schema_version".to_string());

    let candidates =
        contract_repair_candidate_observations(&state, "inspect data/app.sqlite", "", &route);
    let candidate = contract_candidate(&candidates, "sqlite_structured_version");
    assert_candidate(
        candidate,
        "contract:sqlite_schema_version",
        Some("data/app.sqlite"),
        Some("scalar"),
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn sqlite_structured_table_selector_repairs_to_table_listing_contract() {
    let root = make_temp_root("sqlite_structured_table_listing_repair");
    std::fs::create_dir_all(root.join("data")).expect("data directory");
    std::fs::write(root.join("data/test_contract.sqlite"), "").expect("sqlite file");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "data/test_contract.sqlite".to_string();
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("sqlite.tables".to_string());

    let candidates = contract_repair_candidate_observations(
        &state,
        "inspect data/test_contract.sqlite",
        "",
        &route,
    );
    let candidate = contract_candidate(&candidates, "sqlite_structured_table_listing");
    assert_candidate(
        candidate,
        "contract:sqlite_table_listing",
        Some("data/test_contract.sqlite"),
        Some("strict"),
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn sqlite_route_reason_table_token_repairs_raw_command_contract() {
    let root = make_temp_root("sqlite_route_reason_table_repair");
    std::fs::create_dir_all(root.join("data")).expect("data directory");
    std::fs::write(root.join("data/test_contract.sqlite"), "").expect("sqlite file");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.route_reason =
        "sqlite_table_listing; semantic_contract_requires_evidence; explicit_command_requires_fresh_execution"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();

    let candidates = contract_repair_candidate_observations(
        &state,
        "Run read-only SQLite query `SELECT name FROM sqlite_master WHERE type='table'` on data/test_contract.sqlite",
        "",
        &route,
    );
    let candidate = contract_candidate(&candidates, "sqlite_route_reason_table");
    assert_candidate(
        candidate,
        "contract:sqlite_table_listing",
        Some("data/test_contract.sqlite"),
        Some("strict"),
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn config_validation_findings_selector_repairs_to_config_risk_contract() {
    let root = make_temp_root("config_validation_findings_repair");
    std::fs::create_dir_all(root.join("configs")).expect("configs directory");
    std::fs::write(
        root.join("configs/config.toml"),
        "[llm]\nselected_vendor = \"minimax\"\n",
    )
    .expect("config file");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("config_validation_findings".to_string());

    let candidates =
        contract_repair_candidate_observations(&state, "inspect configs/config.toml", "", &route);
    let candidate = contract_candidate(&candidates, "config_validation_findings");
    assert_candidate(
        candidate,
        "contract:config_risk_assessment",
        Some("configs/config.toml"),
        Some("free"),
    );

    let _ = std::fs::remove_dir_all(root);
}

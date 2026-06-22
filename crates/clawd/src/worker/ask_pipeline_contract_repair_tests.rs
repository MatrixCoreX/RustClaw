use super::{
    repair_compound_file_names_plus_content_summary_contract,
    repair_generic_path_content_grounded_summary_contract,
    repair_session_alias_listing_plus_content_summary_contract,
    repair_sqlite_path_excerpt_judgment_contract,
    repair_summary_only_content_excerpt_with_summary_contract,
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

fn route_reason_has_marker(route_result: &crate::RouteResult, marker: &str) -> bool {
    super::super::route_reason_has_marker(route_result, marker)
}

#[test]
fn compound_file_names_plus_content_summary_repair_relaxes_exact_names_contract() {
    let mut route = executable_filename_route();
    route.route_reason =
        "llm_semantic_contract_repair:malformed_contract_listing_vs_content_synthesis_conflict:repair note"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    repair_compound_file_names_plus_content_summary_contract(&mut route);

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
    assert!(route_reason_has_marker(
        &route,
        "compound_file_names_plus_content_summary_contract_repaired"
    ));
}

#[test]
fn session_alias_directory_and_file_targets_relax_names_contract() {
    let root = make_temp_root("alias_compound_listing_content");
    let dir_target = root.join("archive");
    let file_target = root.join("release_checklist.md");
    std::fs::create_dir_all(&dir_target).expect("archive dir");
    std::fs::write(dir_target.join("README.txt"), "archive note\n").expect("archive readme");
    std::fs::write(&file_target, "release checklist\n").expect("checklist");
    let state = test_state_with_root(root.clone());
    let snapshot = alias_snapshot(&dir_target, &file_target);
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    repair_session_alias_listing_plus_content_summary_contract(
        &state,
        "alpha_dir beta_file",
        &snapshot,
        &mut route,
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
    assert!(route_reason_has_marker(
        &route,
        "session_alias_listing_plus_content_summary_contract_repaired"
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_alias_directory_and_file_targets_relax_directory_group_contract() {
    let root = make_temp_root("alias_compound_directory_group_content");
    let dir_target = root.join("archive");
    let file_target = root.join("release_checklist.md");
    std::fs::create_dir_all(&dir_target).expect("archive dir");
    std::fs::write(dir_target.join("README.txt"), "archive note\n").expect("archive readme");
    std::fs::write(&file_target, "release checklist\n").expect("checklist");
    let state = test_state_with_root(root.clone());
    let snapshot = alias_snapshot(&dir_target, &file_target);
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    repair_session_alias_listing_plus_content_summary_contract(
        &state,
        "alpha_dir beta_file",
        &snapshot,
        &mut route,
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
    assert!(route_reason_has_marker(
        &route,
        "session_alias_listing_plus_content_summary_contract_repaired"
    ));
    let _ = std::fs::remove_dir_all(root);
}

fn alias_snapshot(
    dir_target: &std::path::Path,
    file_target: &std::path::Path,
) -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![
                crate::conversation_state::SessionAliasBinding {
                    alias: "alpha_dir".to_string(),
                    target: dir_target.display().to_string(),
                    updated_at_ts: 1,
                },
                crate::conversation_state::SessionAliasBinding {
                    alias: "beta_file".to_string(),
                    target: file_target.display().to_string(),
                    updated_at_ts: 2,
                },
            ],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

#[test]
fn compound_file_paths_plus_content_summary_repair_uses_content_summary_contract() {
    let mut route = executable_filename_route();
    route.route_reason =
        "llm_semantic_contract_repair:compound_file_paths_requires_structured_summary".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    repair_compound_file_names_plus_content_summary_contract(&mut route);

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route_reason_has_marker(
        &route,
        "compound_file_paths_summary_bound_to_current_workspace"
    ));
    assert!(route_reason_has_marker(
        &route,
        "compound_file_paths_plus_content_summary_contract_repaired"
    ));
}

#[test]
fn strict_file_paths_contract_does_not_repair_to_content_summary() {
    let mut route = executable_filename_route();
    route.route_reason =
        "llm_semantic_contract_repair:compound_file_paths_requires_structured_summary".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    repair_compound_file_names_plus_content_summary_contract(&mut route);

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FilePaths
    );
    assert!(!route_reason_has_marker(
        &route,
        "compound_file_paths_plus_content_summary_contract_repaired"
    ));
}

#[test]
fn summary_only_content_excerpt_with_summary_repair_uses_summary_contract() {
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;

    repair_summary_only_content_excerpt_with_summary_contract(&mut route);

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert!(route_reason_has_marker(
        &route,
        "summary_only_content_excerpt_with_summary_contract_repaired"
    ));
}

#[test]
fn summary_only_content_excerpt_with_summary_repair_preserves_strict_excerpt_plus_summary() {
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    repair_summary_only_content_excerpt_with_summary_contract(&mut route);

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptWithSummary
    );
    assert!(!route_reason_has_marker(
        &route,
        "summary_only_content_excerpt_with_summary_contract_repaired"
    ));
}

#[test]
fn generic_path_content_grounded_summary_repair_uses_content_summary_contract() {
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;

    let repaired = repair_generic_path_content_grounded_summary_contract(&mut route);

    assert!(repaired);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert!(route_reason_has_marker(
        &route,
        "generic_path_content_grounded_summary_contract_repaired"
    ));
}

#[test]
fn generic_path_content_repair_preserves_command_summary_marker_contract() {
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

    let repaired = repair_generic_path_content_grounded_summary_contract(&mut route);

    assert!(repaired);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::CommandOutputSummary
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(route_reason_has_marker(
        &route,
        "command_observation_marker_contract_repaired"
    ));
    assert!(!route_reason_has_marker(
        &route,
        "generic_path_content_grounded_summary_contract_repaired"
    ));
}

#[test]
fn generic_path_content_grounded_summary_repair_preserves_strict_contract() {
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    let repaired = repair_generic_path_content_grounded_summary_contract(&mut route);

    assert!(!repaired);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(!route_reason_has_marker(
        &route,
        "generic_path_content_grounded_summary_contract_repaired"
    ));
}

#[test]
fn sqlite_path_excerpt_judgment_contract_repair_uses_db_kind() {
    let root = make_temp_root("sqlite_path_excerpt_judgment_repair");
    std::fs::create_dir_all(root.join("data")).expect("data directory");
    std::fs::write(root.join("data/db-basic-contract.sqlite"), "").expect("sqlite file");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "data/db-basic-contract.sqlite".to_string();

    assert!(repair_sqlite_path_excerpt_judgment_contract(
        &state,
        "inspect data/db-basic-contract.sqlite",
        "",
        &mut route,
    ));

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::SqliteDatabaseKindJudgment
    );
    assert!(route
        .output_contract
        .locator_hint
        .ends_with("data/db-basic-contract.sqlite"));
    assert!(route_reason_has_marker(
        &route,
        "sqlite_path_excerpt_judgment_contract_repaired"
    ));
    let policy = crate::contract_matrix::action_policy_for_output_contract(
        Some(&route.output_contract),
        "db_basic",
        &serde_json::json!({
            "action": "list_tables",
            "db_path": route.output_contract.locator_hint,
        }),
    )
    .expect("db_basic policy");
    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.contract_match, "sqlite_database_kind_judgment");

    let _ = std::fs::remove_dir_all(root);
}

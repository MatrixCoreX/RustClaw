use super::super::deictic_missing_locator_reason_code;
use super::super::route_reason_has_marker;
use super::{
    promote_broad_current_workspace_content_summary_to_directory_purpose,
    repair_directory_purpose_command_summary_contract,
    repair_directory_purpose_quantity_comparison_contract,
    restore_explicit_extension_assess_gap_to_command_summary,
    unbound_model_context_target_route_should_force_clarify,
    unbound_targeted_evidence_route_should_force_clarify,
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
        "rustclaw_unbound_context_guard_{label}_{}_{}",
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
fn unbound_current_workspace_count_requires_clarify_without_anchor() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(unbound_targeted_evidence_route_should_force_clarify(
        "count the requested target's direct children and output only the number",
        &route,
        &snapshot,
        "<none>",
    ));
    assert_eq!(
        deictic_missing_locator_reason_code(&route),
        "missing_count_target"
    );
}

#[test]
fn bound_current_workspace_count_does_not_trigger_unbound_fallback_guard() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/tmp/rustclaw".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "count direct children in the current workspace and output only the number",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn current_workspace_scope_marker_allows_root_hint_scalar_count() {
    let root = make_temp_root("current_workspace_scope_marker_root_hint");
    let mut route = executable_filename_route();
    route.route_reason =
        "semantic_contract_requires_evidence; current_workspace_scope_from_current_request"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root.display().to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "count direct workspace children and output only the number",
        &route,
        &snapshot,
        "<none>",
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn bound_current_workspace_content_summary_does_not_trigger_unbound_fallback_guard() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/tmp/rustclaw".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "summarize the bound current workspace using evidence",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn current_workspace_hidden_entries_check_does_not_trigger_unbound_fallback_guard() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::HiddenEntriesCheck;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "check hidden entries in the current workspace and list examples",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn current_workspace_scalar_equality_check_does_not_trigger_unbound_fallback_guard() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "check whether the current git branch equals main",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn unbound_scalar_count_without_locator_requires_clarify() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(unbound_targeted_evidence_route_should_force_clarify(
        "count direct children and output only the number",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn unbound_current_workspace_file_summary_requires_clarify_without_anchor() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(unbound_targeted_evidence_route_should_force_clarify(
        "read the beginning of the requested documentation and summarize it in one sentence",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn broad_current_workspace_content_summary_repairs_to_directory_purpose_summary() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        promote_broad_current_workspace_content_summary_to_directory_purpose(
            "summarize the current workspace structure in one sentence",
            &mut route,
        )
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryPurposeSummary
    );
    assert!(route
        .route_reason
        .contains("broad_current_workspace_content_summary_repaired_to_directory_purpose_summary"));
}

#[test]
fn concrete_current_workspace_content_summary_does_not_repair_to_directory_purpose_summary() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        !promote_broad_current_workspace_content_summary_to_directory_purpose(
            "read the beginning of README.md and summarize it in one sentence",
            &mut route,
        )
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
}

#[test]
fn command_summary_with_directory_purpose_marker_repairs_to_directory_purpose() {
    let state = test_state_with_root(make_temp_root("directory_purpose_command_repair"));
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "prompts/schemas".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::CommandOutputSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.route_reason =
        "llm_semantic_contract_repair:request_needs_directory_purpose_synthesis_plus_comparative_selection"
            .to_string();

    assert!(repair_directory_purpose_command_summary_contract(
        &state,
        "summarize prompts/schemas schema inventory and purpose",
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryPurposeSummary
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
    assert!(route
        .route_reason
        .contains("command_summary_repaired_to_directory_purpose_summary"));
}

#[test]
fn explicit_command_summary_does_not_repair_to_directory_purpose() {
    let state = test_state_with_root(make_temp_root("explicit_command_summary_no_repair"));
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "prompts/schemas".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::CommandOutputSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.route_reason = concat!(
        "llm_semantic_contract_repair:request_needs_directory_purpose_synthesis_plus_comparative_selection",
        "; explicit_command_requires_command_output_summary_execution"
    )
    .to_string();

    assert!(!repair_directory_purpose_command_summary_contract(
        &state,
        "run command: ls -laS prompts/schemas/*.json | head -20 and summarize it",
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::CommandOutputSummary
    );
}

#[test]
fn extension_assess_gap_command_summary_does_not_repair_to_directory_purpose() {
    let state = test_state_with_root(make_temp_root("extension_gap_no_directory_repair"));
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::CommandOutputSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "skill=extension_manager action=assess_gap".to_string();
    route.route_reason = concat!(
        "llm_semantic_contract_repair:request_needs_directory_purpose_synthesis_plus_comparative_selection",
        "; capability=extension.assess_gap"
    )
    .to_string();

    assert!(!repair_directory_purpose_command_summary_contract(
        &state,
        "extension_manager assess_gap",
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::CommandOutputSummary
    );
}

#[test]
fn extension_assess_gap_directory_contract_restores_to_command_summary() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.clarify_question = "clarify".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "*.csv".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "skill=extension_manager action=assess_gap".to_string();
    route.route_reason = "capability=extension.assess_gap".to_string();

    assert!(restore_explicit_extension_assess_gap_to_command_summary(
        &mut route
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::CommandOutputSummary
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
    assert!(route
        .route_reason
        .contains("extension_assess_gap_contract_restored_to_command_output_summary"));
    assert!(!route.needs_clarify);
    assert!(route.clarify_question.is_empty());
    assert!(route.is_execute_gate());
}

#[test]
fn quantity_comparison_single_directory_repair_promotes_to_directory_purpose() {
    let root = make_temp_root("directory_purpose_quantity_repair");
    std::fs::create_dir_all(root.join("prompts/schemas")).expect("schemas");
    std::fs::write(
        root.join("prompts/schemas/intent_normalizer.schema.json"),
        r#"{"title":"IntentNormalizerOut"}"#,
    )
    .expect("schema");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "prompts/schemas".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.route_reason =
        "llm_semantic_contract_repair:request_needs_directory_purpose_synthesis_plus_comparative_selection"
            .to_string();

    assert!(repair_directory_purpose_quantity_comparison_contract(
        &state,
        "inventory prompts/schemas/*.json and synthesize object purpose",
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryPurposeSummary
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
    assert!(route_reason_has_marker(
        &route,
        "quantity_comparison_repaired_to_directory_purpose_summary"
    ));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quantity_comparison_path_pair_does_not_repair_to_directory_purpose() {
    let root = make_temp_root("directory_purpose_quantity_pair_no_repair");
    std::fs::create_dir_all(root.join("left")).expect("left");
    std::fs::create_dir_all(root.join("right")).expect("right");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = format!(
        "{} | {}",
        root.join("left").display(),
        root.join("right").display()
    );
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.route_reason =
        "llm_semantic_contract_repair:request_needs_directory_purpose_synthesis_plus_comparative_selection"
            .to_string();

    assert!(!repair_directory_purpose_quantity_comparison_contract(
        &state,
        "compare two directories",
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::QuantityComparison
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unbound_current_workspace_project_summary_still_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "summarize the current workspace structure in one sentence",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn unbound_current_workspace_directory_purpose_summary_still_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "summarize the purpose of top-level config files in the current workspace",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn unbound_current_workspace_file_paths_search_still_allows_execution() {
    let state = test_state_with_root(make_temp_root("current_workspace_file_paths"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Find 5 representative toml files in the current repository and output paths".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "find 5 representative toml files in the current repository and output paths",
        &route,
        None,
        &snapshot,
    ));
    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "find 5 representative toml files in the current repository and output paths",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn current_workspace_file_paths_search_ignores_cjk_count_phrase_as_locator_target() {
    let state = test_state_with_root(make_temp_root("current_workspace_file_paths_cjk_count"));
    let mut route = executable_filename_route();
    route.resolved_intent = "在当前仓库找出5个代表性的 toml 文件，只输出路径列表".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "找出仓库里 5 个代表性的 toml 文件，只输出路径列表",
        &route,
        None,
        &snapshot,
    ));
    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "找出仓库里 5 个代表性的 toml 文件，只输出路径列表",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn unbound_current_workspace_semantic_none_allows_self_scoped_observation() {
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Detect which package manager is present in the current workspace.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "Which package manager is detected for this workspace?",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn unbound_model_context_allows_current_workspace_generic_observation() {
    let state = test_state_with_root(make_temp_root("unbound_model_current_workspace_generic"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Preview how files in the current workspace could be categorized.".to_string();
    route.route_reason =
        "The request needs observing the current workspace before summarizing.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "preview the current workspace categories",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_allows_current_workspace_directory_purpose_summary() {
    let state = test_state_with_root(make_temp_root(
        "unbound_model_current_workspace_directory_purpose",
    ));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "List matching top-level workspace entries and summarize their purpose.".to_string();
    route.route_reason =
        "The contract targets the current workspace directory listing.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "list matching workspace entries and explain their purpose",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_target_requires_clarify_before_planner_guess() {
    let state = test_state_with_root(make_temp_root("unbound_model_context_target"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Extract the name field from the package manifest (Cargo.toml).".to_string();
    route.route_reason =
        "User requests the package name from the package file; Cargo.toml can be read directly."
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(unbound_model_context_target_route_should_force_clarify(
        &state,
        "extract name from that package file",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn task_control_machine_route_executes_without_locator() {
    let state = test_state_with_root(make_temp_root("task_control_without_locator"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Use task_control.list and task_control.get to inspect task lifecycle fields.".to_string();
    route.route_reason = "task_control.list task_control.get runtime task query".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "inspect task lifecycle fields",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_target_allows_inline_csv_transform_payload() {
    let state = test_state_with_root(make_temp_root("unbound_model_context_inline_csv"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Sort the embedded CSV records and render a markdown table.".to_string();
    route.route_reason = "inline structured records can be transformed directly".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "这个 CSV 按 score 降序输出 markdown 表格：name,score\\nli,3\\nwang,8\\nzhao,5",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_target_allows_configured_raw_command_without_locator() {
    let mut state = test_state_with_root(make_temp_root("unbound_model_context_raw_command"));
    state.policy.command_intent.execute_prefixes = vec!["run ".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string()];
    let mut route = executable_filename_route();
    route.resolved_intent = "Get current working directory path".to_string();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
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

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "Run pwd and output only the raw result.",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_target_ignores_reason_only_example_targets() {
    let state = test_state_with_root(make_temp_root("unbound_model_context_reason_example"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Detect which package manager is used in this workspace.".to_string();
    route.route_reason =
        "Observation may inspect examples such as Cargo.toml or package.json.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "Which package manager is detected for this workspace?",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_target_allows_current_turn_locator() {
    let root = make_temp_root("unbound_model_context_current_locator");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Demo\n").expect("readme");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "Read README.md and summarize it.".to_string();
    route.route_reason = "README.md is the requested target.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "README.md",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_target_allows_current_workspace_listing_scope() {
    let root = make_temp_root("unbound_model_context_current_workspace_listing");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.resolved_intent = format!(
        "列出当前仓库({})顶层目录和文件，按文件/目录简单分组",
        root.display()
    );
    route.route_reason = "structured current workspace listing request".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "把当前仓库顶层目录和文件列出来，简单分组就行",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_target_allows_current_workspace_file_path_inventory_scope() {
    let root = make_temp_root("unbound_model_context_current_workspace_file_paths");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.resolved_intent = format!(
        "Find five representative TOML file paths in the current workspace ({})",
        root.display()
    );
    route.route_reason = "semantic_contract_requires_evidence".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "找出仓库里 5 个代表性的 toml 文件，只输出路径列表",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_target_allows_current_workspace_recent_artifact_judgment_scope() {
    let root = make_temp_root("unbound_model_context_current_workspace_recent_artifacts");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Find TOML files in the current workspace and summarize representative entries".to_string();
    route.route_reason = concat!(
        "llm_semantic_contract_repair:repair_to_recent_artifacts_judgment_for_discovery_plus_brief_synthesis",
        "; semantic_contract_requires_evidence"
    )
    .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "find toml files in this repo and briefly mention a few representative ones",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn unbound_model_context_target_still_rejects_other_unmentioned_workspace_path() {
    let root = make_temp_root("unbound_model_context_other_workspace");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.resolved_intent = format!(
        "列出 {}/other-project 顶层目录和文件，按文件/目录简单分组",
        root.display()
    );
    route.route_reason = "structured listing request with unmentioned path".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(unbound_model_context_target_route_should_force_clarify(
        &state,
        "把当前仓库顶层目录和文件列出来，简单分组就行",
        &route,
        None,
        &snapshot,
    ));
}

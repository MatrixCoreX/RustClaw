use super::super::deictic_guard::deictic_missing_locator_reason_code;
use super::{
    task_control_route_can_plan_without_locator,
    unbound_model_context_target_route_should_defer_to_agent_loop,
    unbound_targeted_evidence_route_should_defer_to_agent_loop,
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
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
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
    route.route_reason = "target_locator_required".to_string();
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

    assert!(unbound_targeted_evidence_route_should_defer_to_agent_loop(
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
fn current_workspace_count_semantic_enum_alone_does_not_force_preloop_clarify() {
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "count the current workspace entries and output only the number",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn locatorless_content_evidence_with_active_bound_target_reaches_agent_loop() {
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "summarize current result",
        &route,
        &snapshot,
        "<none>",
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None,
        "active target should reach planner through loop observations, not route mutation"
    );
    assert!(route.output_contract.locator_hint.is_empty());
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "summarize the bound current workspace using evidence",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn current_workspace_locator_kind_does_not_force_unbound_clarify() {
    let mut route = executable_filename_route();
    route.resolved_intent = "capability_ref=git.status locator_hint=/tmp/rustclaw".to_string();
    route.route_reason = "capability_ref=git.status".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/tmp/rustclaw".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "report current workspace status fields",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn unmarked_resolved_only_workspace_hint_still_forces_unbound_clarify() {
    let mut route = executable_filename_route();
    route.resolved_intent = "read /tmp/other-project and summarize".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/tmp/other-project".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "summarize current workspace evidence",
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
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

    assert!(unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "count direct children and output only the number",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn unbound_current_workspace_file_summary_requires_clarify_without_anchor() {
    let mut route = executable_filename_route();
    route.route_reason = "target_locator_required".to_string();
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

    assert!(unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "read the beginning of the requested documentation and summarize it in one sentence",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn runtime_surface_clawcli_resume_is_not_promoted_by_worker_post_route() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.clarify_question = "clarify".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent =
        "capability_ref=clawcli.resume action=resume field=resume_task_id field=status".to_string();

    assert!(route.needs_clarify);
    assert_eq!(route.clarify_question, "clarify");
    assert!(!route.is_execute_gate());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Strict
    );

    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "clawcli resume resume_task_id",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn runtime_surface_prompt_only_token_remains_worker_neutral() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.clarify_question = "clarify".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "Inspect runtime resume fields".to_string();

    assert!(route.needs_clarify);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "find 5 representative toml files in the current repository and output paths",
            &route,
            None,
            &snapshot,
        )
    );
    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "找出仓库里 5 个代表性的 toml 文件，只输出路径列表",
            &route,
            None,
            &snapshot,
        )
    );
    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
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

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "preview the current workspace categories",
            &route,
            None,
            &snapshot,
        )
    );
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "list matching workspace entries and explain their purpose",
            &route,
            None,
            &snapshot,
        )
    );
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

    assert!(
        unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "extract name from that package file",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn unbound_model_context_allows_kb_namespace_capability_without_locator() {
    let state = test_state_with_root(make_temp_root("unbound_model_kb_namespace_catalog"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "capability_ref=kb.list_namespaces field=names field=count limit=10".to_string();
    route.route_reason =
        "structured non-filesystem metadata query; capability_ref=kb.list_namespaces".to_string();
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "list current knowledge-base namespaces",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn unbound_targeted_evidence_allows_kb_namespace_capability_without_locator() {
    let mut route = executable_filename_route();
    route.resolved_intent =
        "capability_ref=kb.list_namespaces field=names field=count limit=10".to_string();
    route.route_reason =
        "structured non-filesystem metadata query; capability_ref=kb.list_namespaces".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "list current knowledge-base namespaces",
        &route,
        &snapshot,
        "",
    ));
}

#[test]
fn unbound_targeted_evidence_allows_kb_search_capability_without_locator() {
    let mut route = executable_filename_route();
    route.resolved_intent =
        "capability_ref=kb.search namespace=docs query=service_status top_k=3".to_string();
    route.route_reason = "structured skill-managed KB search; capability_ref=kb.search".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "search kb namespace",
        &route,
        &snapshot,
        "",
    ));
}

#[test]
fn capability_ref_scalar_count_semantic_enum_does_not_force_unbound_clarify() {
    let mut route = executable_filename_route();
    route.resolved_intent = "capability_ref=kb.list_namespaces field=count".to_string();
    route.route_reason = "capability_ref=kb.list_namespaces".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "count knowledge-base namespaces",
        &route,
        &snapshot,
        "",
    ));
}

#[test]
fn task_control_machine_route_executes_without_locator() {
    let state = test_state_with_root(make_temp_root("task_control_without_locator"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Use task_control.list and task_control.get to inspect task lifecycle fields.".to_string();
    route.route_reason =
        "capability_ref=task_control.list capability_ref=task_control.get".to_string();
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

    assert!(task_control_route_can_plan_without_locator(&route));
    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "inspect task lifecycle fields",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn task_control_text_token_without_capability_ref_does_not_bypass_locator_guard() {
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Use task_control.list and task_control.get to inspect task lifecycle fields.".to_string();
    route.route_reason = "task_control.list task_control.get runtime task query".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentPresenceCheck;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;

    assert!(!task_control_route_can_plan_without_locator(&route));
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "这个 CSV 按 score 降序输出 markdown 表格：name,score\\nli,3\\nwang,8\\nzhao,5",
            &route,
            None,
            &snapshot,
        )
    );
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "Run pwd and output only the raw result.",
            &route,
            None,
            &snapshot,
        )
    );
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "Which package manager is detected for this workspace?",
            &route,
            None,
            &snapshot,
        )
    );
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "README.md",
            &route,
            None,
            &snapshot,
        )
    );
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "把当前仓库顶层目录和文件列出来，简单分组就行",
            &route,
            None,
            &snapshot,
        )
    );
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "找出仓库里 5 个代表性的 toml 文件，只输出路径列表",
            &route,
            None,
            &snapshot,
        )
    );
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

    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "find toml files in this repo and briefly mention a few representative ones",
            &route,
            None,
            &snapshot,
        )
    );
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

    assert!(
        unbound_model_context_target_route_should_defer_to_agent_loop(
            &state,
            "把当前仓库顶层目录和文件列出来，简单分组就行",
            &route,
            None,
            &snapshot,
        )
    );
}

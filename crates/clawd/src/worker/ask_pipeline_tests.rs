use super::{
    apply_ask_post_route, background_only_locator_route_should_force_clarify,
    bare_topic_clarify_question_should_drop_context_target,
    bare_topic_memory_expansion_route_should_force_clarify,
    bare_topic_model_supplied_locator_route_should_force_clarify,
    clarify_fallback_source_or_default, current_workspace_locator_resolution,
    deictic_bare_locator_should_force_clarify, deictic_memory_only_route_should_force_clarify,
    deictic_missing_locator_question, direct_answer_from_structured_anchor_requires_evidence,
    downgrade_background_locator_clarify_to_recent_observed_chat, effective_auto_locator_kind,
    execution_user_request, locatorless_observation_route_should_force_clarify,
    prebind_active_bound_target_from_matching_locator_hint,
    prebind_clarify_workspace_child_locator_from_current_request,
    prebind_direct_file_delivery_locator_before_deictic_guard,
    prebind_existing_workspace_locator_hint_from_current_request,
    prebind_file_delivery_locator_from_recent_ordered_resolved_prompt,
    prebind_quantity_compare_directory_pair_from_current_request,
    prebind_runtime_status_scalar_path_to_current_workspace,
    prebind_session_alias_locator_from_current_request,
    prebind_workspace_child_locator_from_current_request,
    prebind_workspace_child_locator_from_resolved_prompt,
    prebind_workspace_root_locator_from_resolved_prompt,
    preserve_scalar_shape_from_normalizer_candidate_for_clarify,
    promote_locatorless_git_capability_to_repository_state,
    promote_locatorless_scalar_child_metadata_to_quantity_comparison,
    promote_locatorless_scalar_status_query_to_runtime_info,
    promote_locatorless_status_query_to_service_status,
    promote_structured_anchor_direct_answer_to_evidence, route_has_model_supplied_concrete_locator,
    route_reason_has_marker, should_attempt_auto_locator,
    should_preserve_original_inline_structured_input, should_reuse_route_clarify_question,
    should_suppress_recent_execution_in_clarify_context,
    structured_missing_locator_clarify_context, structured_missing_locator_default_question,
    unbound_existing_file_delivery_route_should_force_clarify,
    unbound_model_context_target_route_should_force_clarify,
    unbound_targeted_evidence_route_should_force_clarify,
    WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST,
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
        "rustclaw_ask_pipeline_{label}_{}_{}",
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
            locator_kind: crate::OutputLocatorKind::Filename,
            locator_hint: "README.md".to_string(),
            requires_content_evidence: true,
            ..Default::default()
        },
    }
}

fn unresolved_deictic_analysis() -> crate::intent_router::TurnAnalysis {
    crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "deictic_reference": {"target": "unresolved_prior_object"}
        })),
        attachment_processing_required: false,
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

fn empty_session_snapshot() -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

#[test]
fn llm_failed_normalizer_source_uses_llm_unavailable_fallback_source() {
    assert_eq!(
        clarify_fallback_source_or_default(Some(
            crate::fallback::ClarifyFallbackSource::LlmUnavailable
        )),
        crate::fallback::ClarifyFallbackSource::LlmUnavailable
    );
}

#[test]
fn absent_normalizer_fallback_source_uses_intent_unresolved() {
    assert_eq!(
        clarify_fallback_source_or_default(None),
        crate::fallback::ClarifyFallbackSource::IntentUnresolved
    );
}

#[test]
fn auto_locator_attempts_for_path_locators_even_without_content_evidence() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "读取 Cargo.toml".to_string(),
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
            locator_kind: crate::OutputLocatorKind::Path,
            requires_content_evidence: false,
            ..Default::default()
        },
    };
    assert!(should_attempt_auto_locator(&route));
}

#[test]
fn deictic_bare_locator_forces_clarify_before_auto_locator() {
    let route = executable_filename_route();
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
    );
    assert!(deictic_bare_locator_should_force_clarify(
        &route,
        Some(&analysis)
    ));
}

#[test]
fn deictic_directory_scope_with_target_filename_forces_clarify() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "case_only".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
    );
    assert!(deictic_bare_locator_should_force_clarify(
        &route,
        Some(&analysis)
    ));
}

#[test]
fn deictic_synthesized_relative_path_forces_clarify() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "reports/report.md".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
    );
    assert!(deictic_bare_locator_should_force_clarify(
        &route,
        Some(&analysis)
    ));
}

#[test]
fn deictic_forced_clarify_question_names_missing_locator() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "reports/report.md".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    assert_eq!(
        deictic_missing_locator_question(&route),
        "请提供要搜索的目录或目标文件的具体路径。"
    );
}

#[test]
fn deictic_file_locator_with_filename_hint_still_allows_auto_locator() {
    let route = executable_filename_route();
    assert!(!deictic_bare_locator_should_force_clarify(&route, None));
}

#[test]
fn direct_bare_locator_still_allows_auto_locator() {
    let route = executable_filename_route();
    assert!(!deictic_bare_locator_should_force_clarify(&route, None));
}

#[test]
fn deictic_explicit_path_still_allows_auto_locator() {
    let route = executable_filename_route();
    assert!(!deictic_bare_locator_should_force_clarify(&route, None));
}

#[test]
fn deictic_memory_only_execute_route_requires_clarify_without_session_anchor() {
    let mut route = executable_filename_route();
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    let analysis = unresolved_deictic_analysis();
    assert!(deictic_memory_only_route_should_force_clarify(
        "看看那个目录下面都有什么",
        &route,
        Some(&analysis),
        &snapshot,
    ));
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
    ));
    assert_eq!(
        deictic_missing_locator_question(&route),
        "请提供要计数的具体目录或路径。"
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
    ));
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
    ));
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
fn background_only_locator_rewrite_requires_clarify_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("background_locator_requires_clarify"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "读取 /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md 前 3 行"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md".to_string();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(background_only_locator_route_should_force_clarify(
        &state,
        "读一下那个文件前 3 行",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn ambiguous_filename_token_does_not_prebind_memory_supplied_path_but_allows_root_stem() {
    let root = make_temp_root("ambiguous_filename_token");
    std::fs::write(root.join("README.md"), "# Root\n").expect("root readme");
    let docs_dir = root.join("docs");
    std::fs::create_dir_all(&docs_dir).expect("docs dir");
    let docs_readme = docs_dir.join("README.md");
    std::fs::write(&docs_readme, "# Docs\n").expect("docs readme");
    let state = test_state_with_root(root.clone());
    let docs_readme = docs_readme
        .canonicalize()
        .expect("canonical docs readme")
        .display()
        .to_string();
    let mut route = executable_filename_route();
    route.resolved_intent = format!("Read the beginning of {docs_readme}");
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = docs_readme;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;

    assert!(
        !prebind_existing_workspace_locator_hint_from_current_request(
            &state,
            "读一下 README 开头并总结",
            &mut route,
        )
    );
    assert!(!route
        .route_reason
        .contains(WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST));

    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "读一下 README 开头并总结",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn background_locator_guard_ignores_embedded_answer_candidate_path() {
    let state = test_state_with_root(make_temp_root("background_answer_candidate_path"));
    let mut route = executable_filename_route();
    route.resolved_intent = format!(
        "Return the current runtime scalar\nanswer_candidate: {}",
        state.skill_rt.workspace_root.display()
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let resolved_prompt = route.resolved_intent.clone();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!route_has_model_supplied_concrete_locator(
        &route,
        &resolved_prompt
    ));
    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "return the runtime scalar",
        &resolved_prompt,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn model_supplied_manifest_locator_from_deictic_prompt_requires_clarify() {
    let root = make_temp_root("background_manifest_locator_requires_clarify");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").expect("manifest");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Extract the name field from the package manifest (Cargo.toml).".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "Cargo.toml".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(background_only_locator_route_should_force_clarify(
        &state,
        "extract name from that package file",
        &route.resolved_intent,
        "<none>",
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

#[test]
fn background_only_locator_rewrite_allows_current_turn_filename() {
    let state = test_state_with_root(make_temp_root("background_locator_filename"));
    let mut route = executable_filename_route();
    route.resolved_intent = "读取 README.md 前 3 行".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "README.md",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn model_locator_hint_stem_from_current_request_binds_direct_workspace_file() {
    let root = make_temp_root("structured_locator_hint_stem");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Demo\n").expect("readme");
    std::fs::write(root.join("README.zh-CN.md"), "# Demo zh\n").expect("localized readme");
    std::fs::write(root.join("README_cn.md"), "# Demo cn\n").expect("cn readme");
    let ui_dir = root.join("UI");
    std::fs::create_dir_all(&ui_dir).expect("ui dir");
    std::fs::write(ui_dir.join("README.md"), "# UI\n").expect("nested readme");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "读取项目根目录 README 并用三句话总结。".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let prompt = "读一下 README 然后用恰好三句话总结";
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        prebind_existing_workspace_locator_hint_from_current_request(&state, prompt, &mut route,)
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        readme
            .canonicalize()
            .expect("canonical readme")
            .display()
            .to_string()
    );
    assert!(route_reason_has_marker(
        &route,
        WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST,
    ));
    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        prompt,
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn background_only_locator_rewrite_allows_active_ordered_anchor() {
    let state = test_state_with_root(make_temp_root("background_locator_anchor"));
    let mut route = executable_filename_route();
    route.resolved_intent = "读取 /tmp/work/crates/larkd 前 3 行".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/work/crates/larkd".to_string();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/work/crates".to_string()),
            ordered_entries: vec!["larkd".to_string()],
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "看最后一个的基本信息",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn background_only_locator_rewrite_allows_recent_ordered_entry_anchor() {
    let state = test_state_with_root(make_temp_root("background_locator_recent_ordered"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Send the selected prior listing entry: logs/clawd-dev.log".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs/clawd-dev.log".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
    let recent_execution_context = "\
### RECENT_EXECUTION_EVENTS
- ts=2 kind=ask request=list document result=builtin_write_smoke.txt
full_suite_trace_note.txt
gen-1778122040.png
- ts=1 kind=ask request=list logs result=act_plan.log, clawd-dev.log, clawd.log, clawd.nl-focus.log";
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/work/document".to_string()),
            ordered_entries: vec!["builtin_write_smoke.txt".to_string()],
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "send the selected entry",
        &route.resolved_intent,
        recent_execution_context,
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn background_locator_clarify_downgrades_file_name_judgment_with_recent_results() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.clarify_question = "missing target".to_string();
    route.resolved_intent =
        "Compare two recently observed file excerpts and return the selected filename".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/work/service_notes.md".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.route_reason =
        "semantic judgment already resolved; background_locator_requires_clarify".to_string();
    let recent_execution_context = "\
### RECENT_EXECUTION_EVENTS
- ts=2 kind=ask request=read README.md result=# RustClaw
Chinese version: README.zh-CN.md
- ts=1 kind=ask request=read service_notes.md result=# Service Notes
RustClaw test fixture service notes.";

    assert!(
        downgrade_background_locator_clarify_to_recent_observed_chat(
            &mut route,
            recent_execution_context,
        )
    );
    assert!(!route.needs_clarify);
    assert_eq!(
        route.ask_mode.first_layer_decision(),
        crate::FirstLayerDecision::DirectAnswer
    );
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route
        .route_reason
        .contains("active_observed_output_chat_repair"));
}

#[test]
fn locatorless_observation_requires_clarify_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_requires_clarify"));
    let mut route = executable_filename_route();
    route.resolved_intent = "读取该文件的前 3 行".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(locatorless_observation_route_should_force_clarify(
        &state,
        "读一下那个文件前 3 行",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_rss_news_fetch_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_rss_news_fetch"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Fetch the latest configured RSS news.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RssNewsFetch;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "fetch latest rss news",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_web_search_summary_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_web_search_summary"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Search the web for rust async tutorial and summarize results.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WebSearchSummary;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "search rust async tutorial",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_weather_query_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_weather_query"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Query current weather for Beijing.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WeatherQuery;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "check Beijing weather",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_market_quote_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_market_quote"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Query the latest quote for 600519.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::MarketQuote;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "check 600519 quote",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_image_understanding_can_execute_without_session_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_image_understanding"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Describe the attached image.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ImageUnderstanding;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "describe the attached image",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_scalar_raw_runtime_observation_can_plan_without_state_patch() {
    let state = test_state_with_root(make_temp_root("locatorless_runtime_scalar_raw"));
    let mut route = executable_filename_route();
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

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "runtime scalar status query",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_inline_structured_payload_does_not_require_external_locator() {
    let state = test_state_with_root(make_temp_root("locatorless_inline_payload"));
    let mut route = executable_filename_route();
    route.resolved_intent = r#"Count inline JSON array records."#.to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        r#"统计这个 JSON 数组中对象数量，只输出数字：[{"x":1},{"x":2}]"#,
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn quantity_compare_prebinds_two_workspace_directories_from_current_request() {
    let root = make_temp_root("quantity_dir_pair_prebind");
    std::fs::create_dir_all(root.join("fixtures/tmp/bundle_src")).expect("left");
    std::fs::create_dir_all(root.join("fixtures/tmp/dynamic_guard_unpack_case")).expect("right");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "bundle_src vs dynamic_guard_unpack_case".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
            &mut route,
        )
    );

    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route.output_contract.locator_hint.contains("bundle_src"));
    assert!(route
        .output_contract
        .locator_hint
        .contains("dynamic_guard_unpack_case"));

    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quantity_compare_prebinds_two_workspace_files_from_surface_pair() {
    let root = make_temp_root("quantity_file_pair_prebind");
    std::fs::write(root.join("Cargo.lock"), "lock-data").expect("left");
    std::fs::write(root.join("Cargo.toml"), "toml").expect("right");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我前者大概是后者的几倍",
            &mut route,
        )
    );

    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route.output_contract.locator_hint.contains("Cargo.lock"));
    assert!(route.output_contract.locator_hint.contains("Cargo.toml"));
    assert!(route
        .route_reason
        .contains("quantity_compare_path_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn quantity_compare_does_not_replace_single_existing_directory_locator_with_scanned_pair() {
    let root = make_temp_root("quantity_single_dir_no_pair_override");
    std::fs::create_dir_all(root.join("prompts/schemas")).expect("schemas");
    std::fs::create_dir_all(root.join("patches/open-lark/src/service/search/v2/schema"))
        .expect("other schema dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = root.join("prompts/schemas").display().to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        !prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "列出 prompts/schemas 目录下所有 .json 文件，并找出最大的 schema",
            &mut route,
        )
    );

    assert_eq!(
        route.output_contract.locator_hint,
        root.join("prompts/schemas").display().to_string()
    );
    assert!(!route
        .route_reason
        .contains("quantity_compare_path_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_prebinds_missing_locator_without_forcing_semantic_kind() {
    let root = make_temp_root("directory_pair_missing_locator_prebind");
    std::fs::create_dir_all(root.join("fixtures/tmp/bundle_src")).expect("left");
    std::fs::create_dir_all(root.join("fixtures/tmp/dynamic_guard_unpack_case")).expect("right");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
            &mut route,
        )
    );

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route
        .output_contract
        .locator_hint
        .contains("fixtures/tmp/bundle_src"));
    assert!(route
        .output_contract
        .locator_hint
        .contains("fixtures/tmp/dynamic_guard_unpack_case"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_prebind_skips_archive_locator_pair_contract() {
    let root = make_temp_root("directory_pair_archive_locator_pair");
    std::fs::create_dir_all(root.join("scripts/nl_tests/fixtures/device_local/tmp"))
        .expect("fixture dirs");
    std::fs::create_dir_all(root.join("tmp/contract_matrix_unpacked")).expect("dest dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(!prebind_quantity_compare_directory_pair_from_current_request(
        &state,
        concat!(
            "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。",
            "\n[CONTRACT_TEST_HINT]\n",
            "candidate_wrong_action_ref=fs_basic.write_text\n",
            "policy_expectation=runtime_must_reject_or_replace_disallowed_action\n",
            "[/CONTRACT_TEST_HINT]"
        ),
        &mut route,
    ));

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route
        .route_reason
        .contains("directory_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_prebind_skips_explicit_content_excerpt_contract() {
    let root = make_temp_root("directory_pair_content_excerpt");
    std::fs::create_dir_all(root.join("scripts/nl_tests/fixtures/device_local/docs"))
        .expect("fixture dirs");
    std::fs::create_dir_all(root.join(".git/objects/20")).expect("numeric dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;

    assert!(!prebind_quantity_compare_directory_pair_from_current_request(
        &state,
        concat!(
            "读取 scripts/nl_tests/fixtures/device_local/docs/release_checklist.md 前 20 行，并用三句话总结。",
            "\n[CONTRACT_TEST_HINT]\n",
            "preferred_action_ref=archive_basic.read\n",
            "policy_expectation=use_allowed_action_with_required_evidence\n",
            "[/CONTRACT_TEST_HINT]"
        ),
        &mut route,
    ));

    assert_eq!(
        route.output_contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
    );
    assert!(!route
        .route_reason
        .contains("directory_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_prebind_scan_reaches_late_structural_directory_tokens() {
    let root = make_temp_root("directory_pair_late_structural_scan");
    for idx in 0..2500 {
        std::fs::create_dir_all(root.join(format!("aaa_filler_{idx:04}"))).expect("filler");
    }
    std::fs::create_dir_all(root.join("zz_fixture/tmp/bundle_src")).expect("left");
    std::fs::create_dir_all(root.join("zz_fixture/tmp/dynamic_guard_unpack_case")).expect("right");
    let mut state = test_state_with_root(root.clone());
    state.skill_rt.locator_scan_max_files = 10;
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_quantity_compare_directory_pair_from_current_request(
            &state,
            "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.",
            &mut route,
        )
    );

    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route
        .output_contract
        .locator_hint
        .contains("zz_fixture/tmp/bundle_src"));
    assert!(route
        .output_contract
        .locator_hint
        .contains("zz_fixture/tmp/dynamic_guard_unpack_case"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn locatorless_service_status_observation_does_not_clarify() {
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

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "check whether telegramd is currently running",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_status_query_promotes_to_service_status_before_clarify_guards() {
    let state = test_state_with_root(make_temp_root("locatorless_runtime_status_query"));
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Provide a brief runtime diagnostics overview from fresh system observation.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
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

    assert!(promote_locatorless_status_query_to_service_status(
        &state,
        "status overview",
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus
    );
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "status overview",
        &route,
        Some(&analysis),
        &snapshot,
    ));
    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "status overview",
        &route,
        &snapshot,
    ));
}

#[test]
fn locatorless_status_query_clarify_promotes_to_service_status_execution() {
    let state = test_state_with_root(make_temp_root("locatorless_status_query_clarify"));
    let mut route = executable_filename_route();
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
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

    assert!(promote_locatorless_status_query_to_service_status(
        &state,
        "status overview",
        &mut route,
        Some(&analysis),
    ));

    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus
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
fn runtime_status_scalar_path_binds_current_workspace_before_clarify_guard() {
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

    assert!(prebind_runtime_status_scalar_path_to_current_workspace(
        &mut route,
        Some(&analysis),
        &snapshot,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
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
    assert!(prebind_runtime_status_scalar_path_to_current_workspace(
        &mut route_without_patch,
        Some(&analysis_without_patch),
        &snapshot,
    ));
    assert_eq!(
        route_without_patch.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
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
    assert!(prebind_runtime_status_scalar_path_to_current_workspace(
        &mut route_without_analysis,
        None,
        &snapshot,
    ));
    assert_eq!(
        route_without_analysis.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "return cwd",
        &route_without_analysis,
        None,
        &snapshot,
    ));
}

#[test]
fn scalar_path_with_active_ordered_anchor_without_ref_does_not_bind_current_workspace() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Return only the selected ordered-entry path.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "list matching files".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/rustclaw".to_string()),
            ordered_entries: vec![
                "alpha.txt".to_string(),
                "beta.txt".to_string(),
                "gamma.txt".to_string(),
            ],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!prebind_runtime_status_scalar_path_to_current_workspace(
        &mut route,
        Some(&analysis),
        &snapshot,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route
        .route_reason
        .contains("scalar_path_only_missing_ordered_entry_ref_not_bound_to_current_workspace"));
}

#[test]
fn locatorless_raw_status_query_promotes_when_no_literal_command() {
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

    assert!(promote_locatorless_status_query_to_service_status(
        &state,
        "check whether the local clawd process is present",
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus
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
fn locatorless_status_query_with_explicit_command_does_not_promote_to_service_status() {
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

    assert!(!promote_locatorless_status_query_to_service_status(
        &state,
        "只输出当前机器 hostname",
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
}

#[test]
fn locatorless_status_query_with_command_payload_does_not_promote_to_service_status() {
    let state = test_state_with_root(make_temp_root("locatorless_status_query_command_payload"));
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

    assert!(!promote_locatorless_status_query_to_service_status(
        &state,
        "current runtime user",
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
}

#[test]
fn locatorless_status_query_with_runtime_status_patch_does_not_promote_to_service_status() {
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

    assert!(!promote_locatorless_status_query_to_service_status(
        &state,
        "current runtime user",
        &mut route,
        Some(&analysis),
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
fn scalar_runtime_tool_observation_does_not_promote_to_service_status_without_kind() {
    let state = test_state_with_root(make_temp_root("scalar_runtime_tool_no_kind"));
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

    assert!(!promote_locatorless_status_query_to_service_status(
        &state,
        "runtime scalar",
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
}

#[test]
fn locatorless_scalar_status_query_without_kind_promotes_to_runtime_info() {
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

    assert!(promote_locatorless_scalar_status_query_to_runtime_info(
        &mut route,
        Some(&analysis),
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    assert!(route_reason_has_marker(
        &route,
        "execution_recipe_scalar_runtime_tool_observation"
    ));
    assert!(!promote_locatorless_status_query_to_service_status(
        &state,
        "current runtime scalar",
        &mut route,
        Some(&analysis),
    ));
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
fn locatorless_observation_with_command_payload_raw_output_does_not_clarify() {
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

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "current runtime user",
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
fn unbound_targeted_evidence_allows_current_workspace_scalar_count_scope() {
    let mut route = executable_filename_route();
    route.resolved_intent = "count top-level workspace directories".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "count top-level repository directories",
        &route,
        &snapshot,
    ));
}

#[test]
fn locatorless_git_capability_promotes_to_repository_state_before_clarify_guards() {
    let state = test_state_with_root(make_temp_root("locatorless_git_capability"));
    let mut route = executable_filename_route();
    route.resolved_intent = "Observe git repository state from the current workspace.".to_string();
    route.route_reason = "This requires git_basic readonly observation.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(promote_locatorless_git_capability_to_repository_state(
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::GitRepositoryState
    );
    assert!(!locatorless_observation_route_should_force_clarify(
        &state, "git", &route, None, &snapshot,
    ));
    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "git", &route, &snapshot,
    ));
}

#[test]
fn visible_git_skill_candidate_does_not_steal_process_status_route() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Observe local listening ports from the host system.".to_string();
    route.route_reason = "This requires local process status observation.".to_string();
    route.visible_skill_candidates = vec!["process_basic".to_string(), "git_basic".to_string()];
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    assert!(!promote_locatorless_git_capability_to_repository_state(
        &mut route,
    ));
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
}

#[test]
fn locatorless_scalar_child_metadata_promotes_to_quantity_comparison() {
    let root = make_temp_root("locatorless_scalar_child_metadata");
    std::fs::create_dir_all(root.join("target")).expect("target dir");
    std::fs::write(root.join("target").join("artifact.bin"), b"artifact").expect("artifact");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(
        promote_locatorless_scalar_child_metadata_to_quantity_comparison(
            &state,
            "inspect target size",
            &mut route,
        )
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::QuantityComparison
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        root.join("target")
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
    assert!(!route.needs_clarify);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn command_payload_raw_output_does_not_promote_to_quantity_comparison() {
    let root = make_temp_root("command_payload_not_quantity_comparison");
    std::fs::create_dir_all(root.join("run")).expect("run dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;

    assert!(
        !promote_locatorless_scalar_child_metadata_to_quantity_comparison(
            &state,
            "please run uname -a and tell me the result",
            &mut route,
        )
    );

    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn locatorless_scalar_child_metadata_preserves_bare_single_token_topic() {
    let root = make_temp_root("locatorless_scalar_bare_topic");
    std::fs::create_dir_all(root.join("target")).expect("target dir");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(
        !promote_locatorless_scalar_child_metadata_to_quantity_comparison(
            &state, "target", &mut route,
        )
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::RawCommandOutput
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn locatorless_observation_binds_existing_workspace_child_from_current_request() {
    let root = make_temp_root("locatorless_workspace_child");
    let prompts_dir = root.join("prompts");
    std::fs::create_dir_all(&prompts_dir).expect("prompts dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "用户希望查看 prompts 目录下前 5 个条目的名称".to_string();
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

    assert!(prebind_workspace_child_locator_from_current_request(
        &state,
        "先列出 prompts 目录下前 5 个条目名称",
        &mut route,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        prompts_dir
            .canonicalize()
            .expect("canonical prompts")
            .display()
            .to_string()
    );
    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "先列出 prompts 目录下前 5 个条目名称",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn clarify_observation_binds_existing_workspace_child_from_current_request() {
    let root = make_temp_root("clarify_current_workspace_child");
    let configs_dir = root.join("configs");
    std::fs::create_dir_all(&configs_dir).expect("configs dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;

    assert!(
        prebind_clarify_workspace_child_locator_from_current_request(
            &state,
            "先列出 configs 目录下前 5 个条目名称",
            &mut route,
        )
    );
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        configs_dir
            .canonicalize()
            .expect("canonical configs")
            .display()
            .to_string()
    );
}

#[test]
fn clarify_path_contract_binds_existing_workspace_child_from_current_request() {
    let root = make_temp_root("clarify_path_contract_workspace_child");
    let logs_dir = root.join("logs");
    std::fs::create_dir_all(&logs_dir).expect("logs dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;

    assert!(
        prebind_clarify_workspace_child_locator_from_current_request(
            &state,
            "先列出 logs 目录下前 5 个文件名",
            &mut route,
        )
    );
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_hint,
        logs_dir
            .canonicalize()
            .expect("canonical logs")
            .display()
            .to_string()
    );
    assert!(route
        .route_reason
        .contains("workspace_child_locator_prebound_from_clarify_current_request"));
}

#[test]
fn locatorless_observation_does_not_bind_bare_workspace_child_topic() {
    let root = make_temp_root("locatorless_bare_workspace_child");
    let logs_dir = root.join("logs");
    std::fs::create_dir_all(&logs_dir).expect("logs dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;

    assert!(!prebind_workspace_child_locator_from_current_request(
        &state, "logs", &mut route,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn clarify_observation_does_not_bind_bare_workspace_child_topic() {
    let root = make_temp_root("clarify_bare_workspace_child");
    let logs_dir = root.join("logs");
    std::fs::create_dir_all(&logs_dir).expect("logs dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;

    assert!(
        !prebind_clarify_workspace_child_locator_from_current_request(&state, "logs", &mut route,)
    );
    assert!(route.needs_clarify);
    assert!(route.is_clarify_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn clarify_observation_binds_workspace_child_when_semantic_kind_is_generic() {
    let root = make_temp_root("clarify_generic_workspace_child");
    let document_dir = root.join("document");
    std::fs::create_dir_all(&document_dir).expect("document dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    assert!(
        prebind_clarify_workspace_child_locator_from_current_request(
            &state,
            "列出 document 目录最近修改的 2 个文件名，只输出文件名",
            &mut route,
        )
    );
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        document_dir
            .canonicalize()
            .expect("canonical document")
            .display()
            .to_string()
    );
}

#[test]
fn clarify_observation_binds_existing_workspace_file_from_current_request_path() {
    let root = make_temp_root("clarify_current_workspace_file");
    let schema_path = root
        .join("prompts")
        .join("schemas")
        .join("direct_answer_gate.schema.json");
    std::fs::create_dir_all(schema_path.parent().expect("schema parent")).expect("schema dir");
    std::fs::write(&schema_path, "{}").expect("schema file");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;

    assert!(
        prebind_clarify_workspace_child_locator_from_current_request(
            &state,
            "prompts/schemas/direct_answer_gate.schema.json",
            &mut route,
        )
    );
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        schema_path
            .canonicalize()
            .expect("canonical schema")
            .display()
            .to_string()
    );
}

#[test]
fn clarify_observation_binds_existing_workspace_child_from_resolved_prompt() {
    let root = make_temp_root("clarify_resolved_workspace_child");
    let configs_dir = root.join("configs");
    std::fs::create_dir_all(&configs_dir).expect("configs dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;

    assert!(prebind_workspace_child_locator_from_resolved_prompt(
        &state,
        &format!("列出 {} 目录下前 5 个条目名称", configs_dir.display()),
        &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        configs_dir
            .canonicalize()
            .expect("canonical configs")
            .display()
            .to_string()
    );
}

#[test]
fn clarify_observation_binds_existing_workspace_file_from_resolved_prompt_path() {
    let root = make_temp_root("clarify_resolved_workspace_file");
    let schema_path = root
        .join("prompts")
        .join("schemas")
        .join("direct_answer_gate.schema.json");
    std::fs::create_dir_all(schema_path.parent().expect("schema parent")).expect("schema dir");
    std::fs::write(&schema_path, "{}").expect("schema file");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;

    assert!(prebind_workspace_child_locator_from_resolved_prompt(
        &state,
        &format!("查看 {} 中的 target enum", schema_path.display()),
        &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        schema_path
            .canonicalize()
            .expect("canonical schema")
            .display()
            .to_string()
    );
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
fn unclassified_workspace_child_keyword_does_not_bypass_background_locator_guard() {
    let root = make_temp_root("workspace_child_keyword_not_locator");
    std::fs::create_dir_all(root.join("target")).expect("target dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "Inspect schema target enum".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "target".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let analysis = unresolved_deictic_analysis();

    assert!(background_only_locator_route_should_force_clarify(
        &state,
        "inspect schema target enum",
        &route.resolved_intent,
        "<none>",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_command_output_still_allows_execution() {
    let mut state = test_state_with_root(make_temp_root("locatorless_raw_command"));
    state.policy.command_intent.execute_prefixes = vec!["execute ".to_string()];
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "execute pwd, then explain what the path means in one sentence",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_command_sequence_output_still_allows_execution() {
    let mut state = test_state_with_root(make_temp_root("locatorless_raw_command_sequence"));
    state.policy.command_intent.execute_prefixes = vec!["执行".to_string()];
    state.policy.command_intent.standalone_commands = vec!["pwd".to_string(), "whoami".to_string()];
    let mut route = executable_filename_route();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "请依次执行 pwd 和 whoami，直接输出两个命令结果，每个结果一行，不要总结",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_command_without_explicit_command_requires_clarify() {
    let state = test_state_with_root(make_temp_root("locatorless_raw_without_command"));
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(locatorless_observation_route_should_force_clarify(
        &state,
        "list directory contents",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn bare_topic_raw_command_with_unmentioned_context_target_forces_clarify() {
    let mut route = executable_filename_route();
    route.resolved_intent = "View logs from the ops_http_repair test suite".to_string();
    route.route_reason =
        "User typed a bare topic and route context mentioned scripts/nl_suite_logs/ops_http_repair"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(bare_topic_memory_expansion_route_should_force_clarify(
        "logs", &route, None, &snapshot,
    ));
}

#[test]
fn bare_topic_raw_command_without_unmentioned_context_target_stays_executable() {
    let mut route = executable_filename_route();
    route.resolved_intent = "execute pwd command".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!bare_topic_memory_expansion_route_should_force_clarify(
        "pwd", &route, None, &snapshot,
    ));
}

#[test]
fn bare_topic_model_supplied_locator_forces_clarify() {
    let mut route = executable_filename_route();
    route.resolved_intent = "View logs from /workspace/logs".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/workspace/logs".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    let snapshot = empty_session_snapshot();
    assert!(
        bare_topic_model_supplied_locator_route_should_force_clarify(
            "logs", &route, None, &snapshot,
        )
    );
}

#[test]
fn bare_topic_resolved_prompt_prebind_still_forces_clarify() {
    let root = make_temp_root("bare_topic_resolved_prompt_prebind");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("config.toml");
    std::fs::write(&config_path, "[skills]\n[skills.skill_switches]\n").expect("write config");
    let state = test_state_with_root(root);
    let task = crate::ClaimedTask {
        task_id: "bare-topic-resolved-prebind".to_string(),
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
    route.needs_clarify = true;
    route.ask_mode = crate::AskMode::clarify();
    route.resolved_intent = "Apply the config change to the resolved config path".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let resolved_prompt = format!(
        "Apply the config change to {}, setting skills.skill_switches.config_edit_nl_plan to true",
        config_path.display()
    );

    let applied = apply_ask_post_route(
        &state,
        &task,
        "apply_temp_config_cn",
        &resolved_prompt,
        "",
        None,
        route,
        resolved_prompt.clone(),
        resolved_prompt.clone(),
    );

    assert!(applied.execution_route_result.needs_clarify);
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("bare_topic_model_supplied_locator_requires_clarify"));
}

#[test]
fn bare_topic_model_supplied_locator_allows_active_clarify_locator_reply() {
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Continue previous clarified request using the supplied target directory".to_string();
    route.route_reason =
        "preserve_active_clarify_output_contract; active_clarify_locator_reply_execute".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "scripts".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    let snapshot = empty_session_snapshot();
    assert!(
        !bare_topic_model_supplied_locator_route_should_force_clarify(
            "scripts", &route, None, &snapshot,
        )
    );
}

#[test]
fn bare_topic_model_supplied_locator_allows_reused_structured_anchor() {
    let mut route = executable_filename_route();
    route.resolved_intent = "List contents of the document subdirectory".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "document".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({"active_selection": {"target": "document"}})),
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            bound_target: Some("/workspace".to_string()),
            ordered_entries: vec!["document".to_string()],
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !bare_topic_model_supplied_locator_route_should_force_clarify(
            "document",
            &route,
            Some(&analysis),
            &snapshot,
        )
    );
}

#[test]
fn bare_topic_model_supplied_locator_keeps_existing_clarify() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.ask_mode = crate::AskMode::clarify();
    route.resolved_intent = "View logs from /workspace/logs".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/workspace/logs".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    let snapshot = empty_session_snapshot();
    assert!(
        bare_topic_model_supplied_locator_route_should_force_clarify(
            "logs", &route, None, &snapshot,
        )
    );
}

#[test]
fn bare_topic_model_supplied_locator_preserves_non_bare_request() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Check target directory size".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/workspace/target".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;

    let snapshot = empty_session_snapshot();
    assert!(
        !bare_topic_model_supplied_locator_route_should_force_clarify(
            "check target size",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn bare_topic_clarify_question_with_unmentioned_context_target_is_sanitized() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.resolved_intent = "View logs".to_string();
    route.clarify_question =
        "Which logs: ops_http_repair, a specific file path, or current process logs?".to_string();

    assert!(bare_topic_clarify_question_should_drop_context_target(
        "logs", &route
    ));
}

#[test]
fn chinese_deictic_delivery_sentence_is_not_treated_as_bare_topic() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.resolved_intent = "把最近提到的文件发给用户".to_string();
    route.clarify_question = "请提供目标文件或目录的具体路径。".to_string();

    assert!(!bare_topic_clarify_question_should_drop_context_target(
        "把那个文件发给我",
        &route
    ));
}

#[test]
fn locatorless_observation_allows_active_structured_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_active_anchor"));
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
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

    assert!(!locatorless_observation_route_should_force_clarify(
        &state,
        "再看前 3 行",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn locatorless_raw_command_transform_requires_structured_input_despite_unstructured_anchor() {
    let state = test_state_with_root(make_temp_root("locatorless_raw_command_input"));
    let mut route = executable_filename_route();
    route.resolved_intent = "transform /tmp/work/README.md as structured data".to_string();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let prompt = "把那个 JSON 数组按 score 排成表格";
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(locatorless_observation_route_should_force_clarify(
        &state, prompt, &route, None, &snapshot,
    ));
}

#[test]
fn deictic_memory_only_command_output_reference_does_not_force_clarify() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.wants_file_delivery = false;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!deictic_memory_only_route_should_force_clarify(
        "执行 pwd，然后用一句话解释这个路径代表什么",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn deictic_forced_clarify_preserves_only_scalar_shape_from_answer_candidate() {
    let mut route = executable_filename_route();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.resolved_intent =
        "Count direct child items in that directory\nanswer_candidate: 6".to_string();

    preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route);

    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(route.resolved_intent.contains("answer_candidate: 6"));
}

#[test]
fn scalar_shape_preservation_ignores_list_like_answer_candidate() {
    let mut route = executable_filename_route();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.resolved_intent =
        "List candidate files\nanswer_candidate: a.log, b.log, c.log".to_string();

    preserve_scalar_shape_from_normalizer_candidate_for_clarify(&mut route);

    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free
    );
}

#[test]
fn deictic_memory_only_guard_allows_current_session_alias_binding() {
    let mut route = executable_filename_route();
    route.output_contract.locator_hint = "/tmp/docs".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "那个目录".to_string(),
                target: "/tmp/docs".to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!deictic_memory_only_route_should_force_clarify(
        "看看那个目录下面都有什么",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn deictic_memory_only_guard_rejects_session_alias_target_resolved_by_normalizer_only() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Read the first 10 lines of /tmp/device/README.md".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "that_file".to_string(),
                target: "/tmp/device/README.md".to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(deictic_memory_only_route_should_force_clarify(
        "把那个文件开头读 10 行",
        &route,
        Some(&turn_analysis_with_state_patch(serde_json::json!({
            "deictic_reference": {"target": "missing_locator"}
        }))),
        &snapshot,
    ));
}

#[test]
fn deictic_memory_only_guard_rejects_stale_observed_target_without_route_match() {
    let mut route = executable_filename_route();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteSchemaVersion;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.resolved_intent =
        "Query the current workspace SQLite database schema version".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some(
                "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/archive"
                    .to_string(),
            ),
            ..Default::default()
        }),
    };

    let analysis = unresolved_deictic_analysis();
    assert!(deictic_memory_only_route_should_force_clarify(
        "看一下那个 sqlite 的 schema version 是多少",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn deictic_memory_only_guard_allows_active_clarify_anchor() {
    let route = executable_filename_route();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Which file?".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: true,
            output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
            semantic_kind: None,
            source_request: "Send the file".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };

    assert!(!deictic_memory_only_route_should_force_clarify(
        "把那个文件发给我",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn deictic_context_bound_path_still_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
            .to_string();
    assert!(!deictic_bare_locator_should_force_clarify(&route, None));
}

#[test]
fn direct_file_delivery_locator_prebinds_directory_before_deictic_guard() {
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

    assert!(prebind_direct_file_delivery_locator_before_deictic_guard(
        &state, "", &mut route
    ));

    assert!(!deictic_bare_locator_should_force_clarify(&route, None));
    assert_eq!(
        route.output_contract.locator_hint,
        root.join("document")
            .canonicalize()
            .expect("canonical document")
            .display()
            .to_string()
    );
    assert!(route
        .route_reason
        .contains("direct_file_delivery_locator_prebound_before_deictic_guard"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn file_delivery_locator_prebinds_from_recent_ordered_resolved_prompt() {
    let root = make_temp_root("delivery_recent_ordered_prebind");
    let logs_dir = root.join("logs");
    std::fs::create_dir_all(&logs_dir).expect("logs dir");
    let target = logs_dir.join("clawd-dev.log");
    std::fs::write(&target, "line\n").expect("target file");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.wants_file_delivery = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let recent_execution_context =
        "- ts=1 kind=ask request=list logs result=act_plan.log, clawd-dev.log, clawd.log";

    assert!(
        prebind_file_delivery_locator_from_recent_ordered_resolved_prompt(
            &state,
            "Send the selected prior logs list entry clawd-dev.log",
            recent_execution_context,
            &mut route,
        )
    );
    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        target
            .canonicalize()
            .expect("canonical target")
            .display()
            .to_string()
    );
    assert!(route
        .route_reason
        .contains("file_delivery_locator_prebound_from_recent_ordered_resolved_prompt"));
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
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;

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
fn structured_anchor_direct_answer_with_derived_candidate_requires_evidence() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("list document entries".to_string()),
            last_primary_task_output: Some("hello.sh".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "list document entries".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("document".to_string()),
            ordered_entries: vec!["hello.sh".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent = concat!(
        "User wants path and type for the observed entry hello.sh.\n",
        "answer_candidate: {\"path\":\"/tmp/hello.sh\",\"type\":\"file\"}"
    )
    .to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(direct_answer_from_structured_anchor_requires_evidence(
        "What is the path and type for that entry?",
        &route,
        &snapshot,
        true,
        None
    ));

    promote_structured_anchor_direct_answer_to_evidence(&mut route);
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route
        .route_reason
        .contains("structured_anchor_direct_answer_requires_evidence"));
}

#[test]
fn structured_anchor_direct_answer_with_exact_observed_candidate_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("list document entries".to_string()),
            last_primary_task_output: Some("hello.sh".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "list document entries".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("document".to_string()),
            ordered_entries: vec!["hello.sh".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent =
        "User wants the observed entry name.\nanswer_candidate: hello.sh".to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(!direct_answer_from_structured_anchor_requires_evidence(
        "What is that entry name?",
        &route,
        &snapshot,
        true,
        None
    ));
}

#[test]
fn structured_anchor_direct_answer_with_resolved_target_basename_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("send selected log file".to_string()),
            last_primary_task_output: Some(
                "FILE:/home/guagua/rustclaw/logs/clawd-dev.log".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "send selected log file".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some("/home/guagua/rustclaw/logs/clawd-dev.log".to_string()),
            ordered_entries: vec!["clawd-dev.log".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some("/home/guagua/rustclaw/logs/clawd-dev.log".to_string()),
            delivery_targets: vec!["/home/guagua/rustclaw/logs/clawd-dev.log".to_string()],
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent = "User only wants the file basename clawd-dev.log".to_string();
    route.output_contract = crate::IntentOutputContract::default();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(!direct_answer_from_structured_anchor_requires_evidence(
        "Only say this file name.",
        &route,
        &snapshot,
        true,
        None
    ));
}

#[test]
fn structured_anchor_direct_answer_with_existing_context_synthesis_candidate_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("read README opening".to_string()),
            last_primary_task_output: Some(
                "# Device Local Fixture\n\nStable local files for regression tests.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read README opening".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/README.md".to_string()),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent = concat!(
        "Summarize the previously displayed README opening.\n",
        "answer_candidate: This README describes stable local fixture files for regression tests."
    )
    .to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(!direct_answer_from_structured_anchor_requires_evidence(
        "Summarize it in one sentence.",
        &route,
        &snapshot,
        true,
        None
    ));
}

#[test]
fn inline_json_followup_does_not_promote_to_workspace_evidence() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("transform records".to_string()),
            last_primary_task_output: Some("waiting for records".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "transform records".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("records".to_string()),
            ordered_entries: vec!["records".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent = concat!(
        "Sort the provided JSON array by score and render a table.\n",
        "answer_candidate: | name | score |"
    )
    .to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(!direct_answer_from_structured_anchor_requires_evidence(
        r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#,
        &route,
        &snapshot,
        true,
        None
    ));
}

#[test]
fn active_text_mutation_with_structured_anchor_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("GET http://127.0.0.1:8787/v1/health".to_string()),
            last_primary_task_output: Some("Service status: reachable (HTTP 200).".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "GET http://127.0.0.1:8787/v1/health".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("http://127.0.0.1:8787/v1/health".to_string()),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent = "Clarify the current request without reading files.".to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(!direct_answer_from_structured_anchor_requires_evidence(
        "A concept label without a concrete target.",
        &route,
        &snapshot,
        true,
        Some(&analysis)
    ));
}

#[test]
fn deictic_result_reference_with_two_named_files_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"comparison_result"}}),
    );
    assert!(!deictic_bare_locator_should_force_clarify(
        &route,
        Some(&analysis)
    ));
}

#[test]
fn deictic_result_reference_after_command_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"current_action_result"}}),
    );

    assert!(!deictic_bare_locator_should_force_clarify(
        &route,
        Some(&analysis)
    ));
}

#[test]
fn deictic_target_before_followup_still_forces_clarify() {
    let route = executable_filename_route();
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}}),
    );
    assert!(deictic_bare_locator_should_force_clarify(
        &route,
        Some(&analysis)
    ));
}

#[test]
fn deictic_directory_reference_after_named_folder_allows_execution() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "docs".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let analysis = turn_analysis_with_state_patch(
        serde_json::json!({"deictic_reference":{"target":"current_turn_locator"}}),
    );

    assert!(!deictic_bare_locator_should_force_clarify(
        &route,
        Some(&analysis)
    ));
}

#[test]
fn auto_locator_skips_non_path_locators() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "今天天气".to_string(),
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
            requires_content_evidence: false,
            ..Default::default()
        },
    };
    assert!(!should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_attempts_for_current_workspace_locator() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前目录是否存在隐藏文件".to_string(),
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
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            requires_content_evidence: false,
            ..Default::default()
        },
    };
    assert!(should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_skips_clarify_with_unbound_workspace_scope() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "检查当前目录".to_string(),
        needs_clarify: true,
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
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    assert!(!should_attempt_auto_locator(&route));

    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "docs".to_string();
    assert!(should_attempt_auto_locator(&route));

    route.output_contract.locator_hint.clear();
    assert!(!should_attempt_auto_locator(&route));
}

#[test]
fn quantity_comparison_current_workspace_without_hint_does_not_auto_locator_to_root() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;

    assert!(!should_attempt_auto_locator(&route));

    route.output_contract.locator_hint = "/tmp/repo/target".to_string();
    assert!(should_attempt_auto_locator(&route));
}

#[test]
fn current_workspace_locator_resolution_prefers_workspace_root() {
    let root = make_temp_root("current_workspace_locator_root");
    std::fs::create_dir_all(root.join("rustclaw")).expect("nested rustclaw dir");
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Write a long introduction for RustClaw".to_string(),
        needs_clarify: false,
        route_reason: "workspace summary".to_string(),
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
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    assert!(matches!(
        super::current_workspace_locator_resolution(&root, &route),
        Some(crate::post_route_policy::LocatorResolution::Direct(path))
            if path == root.display().to_string()
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_workspace_locator_resolution_accepts_absolute_workspace_hint() {
    let root = make_temp_root("current_workspace_locator_abs_root");
    std::fs::write(root.join("rustclaw"), "#!/usr/bin/env bash\n").expect("launcher file");
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Introduce RustClaw as the current project".to_string(),
        needs_clarify: false,
        route_reason: "workspace summary".to_string(),
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
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: root.display().to_string(),
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    assert!(matches!(
        super::current_workspace_locator_resolution(&root, &route),
        Some(crate::post_route_policy::LocatorResolution::Direct(path))
            if path == root.display().to_string()
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_workspace_locator_hint_naming_root_resolves_to_workspace_root() {
    let parent = make_temp_root("current_workspace_locator_root_name");
    let root = parent.join("rustclaw");
    std::fs::create_dir_all(&root).expect("workspace root");
    std::fs::write(root.join("rustclaw"), "#!/usr/bin/env bash\n").expect("same-name child");
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Introduce the current RustClaw project".to_string(),
        needs_clarify: false,
        route_reason: "workspace summary".to_string(),
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
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: "RustClaw".to_string(),
            requires_content_evidence: true,
            ..Default::default()
        },
    };

    assert!(matches!(
        super::current_workspace_locator_resolution(&root, &route),
        Some(crate::post_route_policy::LocatorResolution::Direct(path))
            if path == root.display().to_string()
    ));
    let _ = std::fs::remove_dir_all(parent);
}

#[test]
fn current_workspace_locator_hint_with_target_name_does_not_resolve_to_root() {
    let root = make_temp_root("current_workspace_locator_named_hint");
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 archive 目录下的所有条目".to_string(),
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
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: "archive".to_string(),
            requires_content_evidence: true,
            ..Default::default()
        },
    };

    assert!(current_workspace_locator_resolution(&root, &route).is_none());
    assert_eq!(
        effective_auto_locator_kind(&route),
        crate::OutputLocatorKind::Path
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_workspace_empty_locator_hint_resolves_to_root() {
    let root = make_temp_root("current_workspace_locator_empty_hint");
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出当前工作区".to_string(),
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
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            requires_content_evidence: true,
            ..Default::default()
        },
    };

    assert!(matches!(
        current_workspace_locator_resolution(&root, &route),
        Some(crate::post_route_policy::LocatorResolution::Direct(path))
            if path == root.display().to_string()
    ));
    assert_eq!(
        effective_auto_locator_kind(&route),
        crate::OutputLocatorKind::CurrentWorkspace
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn auto_locator_attempts_for_filename_locators() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读取 README 前 20 行".to_string(),
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
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    assert!(should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_skips_clarify_routes() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "读一下那个 README 开头，然后一句话总结".to_string(),
        needs_clarify: true,
        route_reason: "normalizer requested clarification before execution".to_string(),
        route_confidence: Some(0.95),
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
            locator_kind: crate::OutputLocatorKind::Path,
            requires_content_evidence: true,
            response_shape: crate::OutputResponseShape::OneSentence,
            ..Default::default()
        },
    };
    assert!(!should_attempt_auto_locator(&route));
}

#[test]
fn auto_locator_skips_stateful_ordered_entry_clarify_routes() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "看第二个".to_string(),
        needs_clarify: true,
        route_reason: "stateful_ordered_entry_ambiguous_clarify:content_read:entries=4".to_string(),
        route_confidence: Some(0.97),
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
            requires_content_evidence: true,
            response_shape: crate::OutputResponseShape::Free,
            ..Default::default()
        },
    };
    assert!(!should_attempt_auto_locator(&route));
}

#[test]
fn inline_json_payload_prefers_original_user_request_for_execution() {
    let prompt = r#"把这个 JSON 数组按 score 从高到低排一下，再输出成 markdown 表格：[{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;
    let resolved =
        "Sort the provided JSON array by score in descending order and output as a markdown table";
    assert!(should_preserve_original_inline_structured_input(
        prompt, resolved
    ));
    assert_eq!(execution_user_request(prompt, resolved), prompt);
}

#[test]
fn non_structured_prompt_keeps_resolved_execution_request() {
    let prompt = "帮我检查 telegramd 现在是不是在运行，顺手简短解释状态";
    let resolved = "检查 telegramd 进程当前是否在运行，并简要说明其状态";
    assert!(!should_preserve_original_inline_structured_input(
        prompt, resolved
    ));
    assert_eq!(execution_user_request(prompt, resolved), resolved);
}

fn clarify_route(locator_kind: crate::OutputLocatorKind) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "读取那个文件".to_string(),
        needs_clarify: true,
        route_reason: "need concrete locator".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: "你是指哪个文件？".to_string(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            locator_kind,
            requires_content_evidence: true,
            response_shape: crate::OutputResponseShape::Scalar,
            ..Default::default()
        },
    }
}

#[test]
fn scalar_missing_locator_attaches_structured_context() {
    let route = clarify_route(crate::OutputLocatorKind::Path);
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_read_target"));
    assert!(context.contains("locator_kind: path"));
}

#[test]
fn structured_missing_locator_records_explicit_route_question_as_context() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.clarify_question = "LOCATOR_CLARIFY_PROMPT".to_string();
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_directory_locator"));
    assert!(context.contains("normalizer_clarify_question_candidate"));
}

#[test]
fn content_missing_locator_attaches_structured_context() {
    let mut route = clarify_route(crate::OutputLocatorKind::Path);
    route.clarify_question.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_read_target"));
    assert!(context.contains("response_shape: free"));
}

#[test]
fn directory_lookup_missing_locator_records_directory_case() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.clarify_question.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::DirectoryLookup;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_directory_locator"));
}

#[test]
fn scalar_count_missing_locator_records_count_case() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.clarify_question.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_count_target"));
    assert!(context.contains("semantic_kind: scalar_count"));
}

#[test]
fn delivery_missing_locator_attaches_structured_context() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.clarify_question.clear();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.wants_file_delivery = true;
    let context = structured_missing_locator_clarify_context(&route, &[])
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: missing_file_locator"));
    assert!(context.contains("delivery_required: true"));
}

#[test]
fn structured_missing_file_locator_default_question_is_specific() {
    let root = make_temp_root("missing_file_question");
    let state = test_state_with_root(root.clone());
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.clarify_question.clear();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.wants_file_delivery = true;

    let question = structured_missing_locator_default_question(&state, "zh-CN", &route, &[])
        .expect("structured default question");

    assert!(question.contains("文件完整路径"));
    assert!(!question.contains("没看出"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn structured_missing_sqlite_locator_default_question_uses_semantic_kind() {
    let root = make_temp_root("missing_sqlite_question");
    let state = test_state_with_root(root.clone());
    let mut route = clarify_route(crate::OutputLocatorKind::Path);
    route.clarify_question.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;

    let question = structured_missing_locator_default_question(&state, "en", &route, &[])
        .expect("structured default question");

    assert!(question.contains("SQLite database file"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn structured_missing_locator_default_question_skips_fuzzy_candidates() {
    let root = make_temp_root("missing_fuzzy_question");
    let state = test_state_with_root(root.clone());
    let route = clarify_route(crate::OutputLocatorKind::Filename);
    let candidates = vec!["/tmp/a/Cargo.toml".to_string()];

    assert!(
        structured_missing_locator_default_question(&state, "en", &route, &candidates).is_none()
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn fuzzy_locator_candidates_attach_structured_context() {
    let route = clarify_route(crate::OutputLocatorKind::Filename);
    let candidates = vec![
        "/tmp/a/Cargo.toml".to_string(),
        "/tmp/b/Cargo.toml".to_string(),
    ];
    let context = structured_missing_locator_clarify_context(&route, &candidates)
        .expect("structured clarify context");
    assert!(context.contains("clarify_case: fuzzy_locator_candidates"));
    assert!(context.contains("candidate_1: /tmp/a/Cargo.toml"));
    assert!(context.contains("candidate_2: /tmp/b/Cargo.toml"));
}

#[test]
fn path_scoped_clarify_without_locator_suppresses_recent_execution_context() {
    let route = clarify_route(crate::OutputLocatorKind::Path);
    assert!(should_suppress_recent_execution_in_clarify_context(
        &route,
        &[],
    ));
}

#[test]
fn filename_clarify_without_locator_reuses_specific_router_question() {
    let route = clarify_route(crate::OutputLocatorKind::Filename);
    assert!(should_reuse_route_clarify_question(
        &route,
        crate::post_route_policy::ClarifyReasonKind::MissingPathScopedLocator,
        &[],
    ));
}

#[test]
fn route_reason_text_can_reuse_router_question_when_structured_locator_exists() {
    let mut route = clarify_route(crate::OutputLocatorKind::Filename);
    route.output_contract.locator_hint = "README.md".to_string();
    assert!(should_reuse_route_clarify_question(
        &route,
        crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        &[],
    ));
}

#[test]
fn clarify_with_fuzzy_candidates_keeps_recent_context_available() {
    let route = clarify_route(crate::OutputLocatorKind::Filename);
    assert!(!should_suppress_recent_execution_in_clarify_context(
        &route,
        &["/tmp/a".to_string()],
    ));
}

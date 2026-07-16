use super::super::{
    background_only_locator_route_should_defer_to_agent_loop,
    locatorless_observation_route_should_defer_to_agent_loop,
    unbound_model_context_target_route_should_defer_to_agent_loop,
};
use super::{
    build_loop_context_after_boundary_preflight, current_request_resolves_workspace_child_locator,
    implicit_workspace_file_locator_route_should_defer_to_agent_loop,
    model_completed_workspace_file_locator_hint_should_defer_to_agent_loop,
    path_scoped_locator_guard_can_defer_to_prompt_targets, workspace_root_name_token_present,
    workspace_root_topic_route_should_require_evidence,
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
        "rustclaw_workspace_locator_binding_{label}_{}_{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&path).expect("temp root");
    path
}

fn make_named_workspace_root(label: &str, name: &str) -> PathBuf {
    let root = make_temp_root(label).join(name);
    std::fs::create_dir_all(&root).expect("named workspace root");
    root
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

fn executionless_free_route() -> crate::RouteResult {
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::act_plain();
    route.route_reason = "executionless_finalize_trace_plain".to_string();
    route.wants_file_delivery = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route
}

#[test]
fn workspace_root_topic_free_route_requires_workspace_summary_evidence() {
    let root = make_named_workspace_root("workspace_root_topic_requires_evidence", "rustclaw");
    std::fs::write(root.join("rustclaw"), "#!/usr/bin/env bash\n").expect("project-name script");
    let state = test_state_with_root(root);
    let route = executionless_free_route();

    assert!(workspace_root_topic_route_should_require_evidence(
        &state.skill_rt.workspace_root,
        "Introduce RustClaw",
        &route,
    ));
}

#[test]
fn workspace_root_topic_boundary_uses_output_contract_marker_without_semantic_kind_write() {
    let root = make_named_workspace_root("workspace_root_topic_output_marker", "rustclaw");
    std::fs::write(root.join("README.md"), "# RustClaw\n").expect("readme");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "workspace-root-topic-output-marker".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executionless_free_route();
    route.resolved_intent = "Introduce the current workspace using grounded evidence.".to_string();
    let resolved_intent = route.resolved_intent.clone();

    let applied = build_loop_context_after_boundary_preflight(
        &state,
        &task,
        "Introduce RustClaw",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert_eq!(
        applied.execution_route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("contract:workspace_project_summary"));
    assert!(applied
        .execution_route_result
        .output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary));
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        applied.execution_route_result.output_contract.locator_hint,
        root.display().to_string()
    );
}

#[test]
fn workspace_root_topic_with_explicit_file_locator_stays_specific_locator_route() {
    let root = make_named_workspace_root("workspace_root_topic_explicit_locator", "rustclaw");
    std::fs::write(root.join("README.md"), "# Demo\n").expect("readme");
    let state = test_state_with_root(root);
    let route = executionless_free_route();

    assert!(!workspace_root_topic_route_should_require_evidence(
        &state.skill_rt.workspace_root,
        "Introduce RustClaw README.md",
        &route,
    ));
}

#[test]
fn generic_free_route_without_workspace_root_token_stays_executionless() {
    let root = make_named_workspace_root("workspace_root_topic_absent", "rustclaw");
    let state = test_state_with_root(root);
    let route = executionless_free_route();

    assert!(!workspace_root_topic_route_should_require_evidence(
        &state.skill_rt.workspace_root,
        "Write a two-line poem",
        &route,
    ));
}

#[test]
fn workspace_project_summary_does_not_bind_bare_project_name_child_file() {
    let root = make_temp_root("workspace_summary_no_project_name_child");
    let child = root.join("rustclaw");
    std::fs::write(&child, "#!/usr/bin/env bash\n").expect("project-name script");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Introduce the current workspace from README and workspace configuration.".to_string();
    route.route_reason = "workspace_project_summary".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;

    assert_eq!(
        current_request_resolves_workspace_child_locator(&state, "Introduce RustClaw"),
        Some(
            child
                .canonicalize()
                .expect("canonical child")
                .display()
                .to_string()
        )
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn kb_stats_route_does_not_bind_document_count_to_workspace_child() {
    let root = make_temp_root("kb_stats_no_document_count_locator");
    let document_dir = root.join("document");
    std::fs::create_dir_all(&document_dir).expect("document dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent =
        "capability_ref=kb.stats namespace=nl_basic_skill_100 field=document_count field=chunk_count"
            .to_string();
    route.route_reason = "skill-managed KB stats; capability_ref=kb.stats".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;

    assert_eq!(
        current_request_resolves_workspace_child_locator(
            &state,
            "只返回 namespace、document_count、chunk_count"
        ),
        Some(
            document_dir
                .canonicalize()
                .expect("canonical document")
                .display()
                .to_string()
        )
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn locatorless_observation_does_not_bind_bare_hidden_vcs_control_path() {
    let root = make_temp_root("locatorless_hidden_vcs_control_path");
    std::fs::create_dir_all(root.join(".git")).expect("git dir");
    std::fs::create_dir_all(root.join("crates")).expect("crates dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent =
        "List top-level workspace entries while excluding a VCS control path.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;

    assert!(current_request_resolves_workspace_child_locator(&state, "inspect .git").is_none());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn implicit_workspace_file_locator_without_model_locator_requires_clarify() {
    let root = make_temp_root("implicit_workspace_file_locator");
    std::fs::write(root.join("README.md"), "# Demo\n").expect("readme");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "Read the beginning of a local file and summarize it.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        implicit_workspace_file_locator_route_should_defer_to_agent_loop(
            &state,
            "读一下那个 README 开头并用一句话总结",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn path_scoped_locator_guard_defers_direct_answer_trace_to_prompt_targets() {
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;

    assert!(path_scoped_locator_guard_can_defer_to_prompt_targets(
        "Read README.md and summarize it",
        &route,
    ));
}

#[test]
fn generated_file_delivery_runtime_target_bypasses_implicit_workspace_file_locator_clarify() {
    let root = make_temp_root("generated_delivery_implicit_locator_bypass");
    std::fs::write(root.join("hello.sh"), "#!/bin/sh\n").expect("existing generated file");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.route_reason = "generated_file_delivery_allows_runtime_target".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !implicit_workspace_file_locator_route_should_defer_to_agent_loop(
            &state,
            "Write a shell script that prints hello world, save it, and send me the file.",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn implicit_workspace_directory_locator_still_allows_prebind() {
    let root = make_temp_root("implicit_workspace_directory_locator");
    let prompts_dir = root.join("prompts");
    std::fs::create_dir_all(&prompts_dir).expect("prompts dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "List entries in a local directory.".to_string();
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

    assert!(
        !implicit_workspace_file_locator_route_should_defer_to_agent_loop(
            &state,
            "先列出 prompts 目录下前 5 个条目名称",
            &route,
            None,
            &snapshot,
        )
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_failed_step_does_not_bind_workspace_child_from_current_request() {
    let root = make_temp_root("explicit_command_failed_step_workspace_child");
    let prompts_dir = root.join("prompts");
    std::fs::create_dir_all(&prompts_dir).expect("prompts dir");
    let state = test_state_with_root(root);
    let prompt = "please run `echo prompts` and tell me the failed execution step";
    assert_eq!(
        crate::agent_engine::explicit_command_segment_for_policy(prompt).as_deref(),
        Some("echo prompts")
    );
    let expected_prompts_dir = prompts_dir
        .canonicalize()
        .expect("canonical prompts")
        .display()
        .to_string();
    assert_eq!(
        current_request_resolves_workspace_child_locator(&state, prompt).as_deref(),
        Some(expected_prompts_dir.as_str())
    );

    let mut route = executable_filename_route();
    route.resolved_intent =
        "Run the explicit local command and report the failed execution step.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
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
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!locatorless_observation_route_should_defer_to_agent_loop(
        &state, prompt, &route, None, &snapshot,
    ));
    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state, prompt, &route, None, &snapshot,
        )
    );
}

#[test]
fn command_payload_failed_step_does_not_bind_workspace_child_from_current_request() {
    let root = make_temp_root("command_payload_failed_step_workspace_child");
    let prompts_dir = root.join("prompts");
    std::fs::create_dir_all(&prompts_dir).expect("prompts dir");
    let state = test_state_with_root(root);
    let prompt = "执行 echo prompts，然后报告失败步骤";
    let expected_prompts_dir = prompts_dir
        .canonicalize()
        .expect("canonical prompts")
        .display()
        .to_string();
    assert_eq!(
        current_request_resolves_workspace_child_locator(&state, prompt).as_deref(),
        Some(expected_prompts_dir.as_str())
    );

    let mut route = executable_filename_route();
    route.route_reason = "command_payload_requires_raw_output_execution".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
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
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(locatorless_observation_route_should_defer_to_agent_loop(
        &state, prompt, &route, None, &snapshot,
    ));
    assert!(
        !unbound_model_context_target_route_should_defer_to_agent_loop(
            &state, prompt, &route, None, &snapshot,
        )
    );
}

#[test]
fn archive_unpack_clarify_does_not_bind_destination_directory_as_source_locator() {
    let root = make_temp_root("archive_unpack_missing_source_keeps_clarify");
    let destination = root.join("tmp").join("dynamic_guard_unpack_case");
    std::fs::create_dir_all(&destination).expect("destination dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.route_reason = "archive_unpack_missing_archive_locator_clarify".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveUnpack;

    let prompt = format!(
        "extract the referenced archive into {} and report the result",
        destination.display()
    );

    assert!(current_request_resolves_workspace_child_locator(&state, &prompt).is_some());
    assert!(route.needs_clarify);
    assert!(!route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn locatorless_observation_does_not_bind_bare_workspace_child_topic() {
    let root = make_temp_root("locatorless_bare_workspace_child");
    let logs_dir = root.join("logs");
    std::fs::create_dir_all(&logs_dir).expect("logs dir");
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;

    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn model_locator_hint_stem_from_current_request_exports_workspace_file_evidence() {
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

    let resolved = current_request_resolves_workspace_child_locator(&state, prompt)
        .expect("current request should resolve workspace child");
    assert_eq!(
        resolved,
        readme
            .canonicalize()
            .expect("canonical readme")
            .display()
            .to_string()
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(route.output_contract.locator_hint, "README");
    assert!(!background_only_locator_route_should_defer_to_agent_loop(
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
fn model_locator_hint_with_generic_semantic_kind_stays_loop_evidence() {
    let root = make_temp_root("structured_locator_hint_generic_semantic");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Demo\n").expect("readme");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "Read the requested workspace locator.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let prompt = "inspect README";

    let resolved = current_request_resolves_workspace_child_locator(&state, prompt)
        .expect("current request should resolve workspace child");
    assert_eq!(
        resolved,
        readme
            .canonicalize()
            .expect("canonical readme")
            .display()
            .to_string()
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Filename
    );
    assert_eq!(route.output_contract.locator_hint, "README");
}

#[test]
fn model_completed_file_locator_without_current_request_match_still_requires_clarify() {
    let root = make_temp_root("model_completed_file_without_prompt_match");
    std::fs::write(root.join("README.md"), "# Demo\n").expect("readme");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "Summarize the project overview.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        model_completed_workspace_file_locator_hint_should_defer_to_agent_loop(
            &state,
            "Summarize the project overview in one sentence",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn current_project_root_name_exports_workspace_root_evidence_before_same_name_child() {
    let parent = make_temp_root("workspace_root_name_prebind_parent");
    let root = parent.join("rustclaw");
    std::fs::create_dir_all(&root).expect("workspace root");
    std::fs::write(root.join("rustclaw"), "#!/usr/bin/env bash\n").expect("same-name child");
    std::fs::write(root.join("README.md"), "# RustClaw\n").expect("readme");
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"0.1.7\"\n",
    )
    .expect("cargo");
    let state = test_state_with_root(root.clone());
    let prompt = "把 RustClaw 当成当前项目来介绍，先查证项目 README 和工作区配置，再写三句话";
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.resolved_intent =
        "介绍 RustClaw 项目，先查证 README.md 和工作区配置文件，基于查证内容撰写三句介绍语"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;

    assert!(workspace_root_name_token_present(
        &state.skill_rt.workspace_root,
        prompt,
    ));
    assert!(route.needs_clarify);
    assert!(route.is_clarify_gate());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    let _ = std::fs::remove_dir_all(parent);
}

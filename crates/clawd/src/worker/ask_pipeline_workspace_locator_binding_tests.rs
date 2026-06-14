use super::super::{
    background_only_locator_route_should_force_clarify,
    locatorless_observation_route_should_force_clarify, route_reason_has_marker,
    unbound_model_context_target_route_should_force_clarify,
};
use super::{
    current_request_resolves_workspace_child_locator,
    implicit_workspace_file_locator_route_should_force_clarify,
    model_completed_workspace_file_locator_hint_should_force_clarify,
    prebind_clarify_workspace_child_locator_from_current_request,
    prebind_existing_workspace_locator_hint_from_current_request,
    prebind_workspace_child_locator_from_current_request,
    prebind_workspace_child_locator_from_resolved_prompt,
    prebind_workspace_root_locator_from_current_request,
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
        "rustclaw_workspace_locator_binding_{label}_{}_{}",
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
fn workspace_project_summary_does_not_bind_bare_project_name_child_file() {
    let root = make_temp_root("workspace_summary_no_project_name_child");
    let child = root.join("rustclaw");
    std::fs::write(&child, "#!/usr/bin/env bash\n").expect("project-name script");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Introduce the current workspace from README and workspace configuration.".to_string();
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
    assert!(!prebind_workspace_child_locator_from_current_request(
        &state,
        "Introduce RustClaw",
        &mut route,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!route
        .route_reason
        .contains("workspace_child_locator_prebound_from_current_request"));
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
    assert!(!prebind_workspace_child_locator_from_current_request(
        &state,
        "list top-level entries except .git",
        &mut route,
    ));
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

    assert!(implicit_workspace_file_locator_route_should_force_clarify(
        &state,
        "读一下那个 README 开头并用一句话总结",
        &route,
        None,
        &snapshot,
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

    assert!(!implicit_workspace_file_locator_route_should_force_clarify(
        &state,
        "Write a shell script that prints hello world, save it, and send me the file.",
        &route,
        None,
        &snapshot,
    ));
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

    assert!(!implicit_workspace_file_locator_route_should_force_clarify(
        &state,
        "先列出 prompts 目录下前 5 个条目名称",
        &route,
        None,
        &snapshot,
    ));
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
}

#[test]
fn explicit_command_failed_step_does_not_bind_workspace_child_from_current_request() {
    let root = make_temp_root("explicit_command_failed_step_workspace_child");
    let prompts_dir = root.join("prompts");
    std::fs::create_dir_all(&prompts_dir).expect("prompts dir");
    let mut state = test_state_with_root(root);
    state.policy.command_intent.execute_prefixes = vec!["please run ".to_string()];
    state.policy.command_intent.standalone_commands = vec!["echo".to_string()];
    let prompt = "please run echo prompts and tell me the failed execution step";
    assert_eq!(
        crate::agent_engine::explicit_command_segment_for_policy(
            &state.policy.command_intent,
            prompt,
        )
        .as_deref(),
        Some("echo")
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

    assert!(!prebind_workspace_child_locator_from_current_request(
        &state, prompt, &mut route,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!locatorless_observation_route_should_force_clarify(
        &state, prompt, &route, None, &snapshot,
    ));
    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state, prompt, &route, None, &snapshot,
    ));
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

    assert!(!prebind_workspace_child_locator_from_current_request(
        &state, prompt, &mut route,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(!locatorless_observation_route_should_force_clarify(
        &state, prompt, &route, None, &snapshot,
    ));
    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state, prompt, &route, None, &snapshot,
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
fn clarify_path_contract_binds_directory_scope_with_child_filename_token() {
    let root = make_temp_root("clarify_path_contract_dir_scope_child_filename");
    let logs_dir = root.join("logs");
    std::fs::create_dir_all(&logs_dir).expect("logs dir");
    std::fs::write(logs_dir.join("clawd.run.log"), "line\n").expect("log file");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;

    assert!(
        prebind_clarify_workspace_child_locator_from_current_request(
            &state,
            "logs clawd.run.log",
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
}

#[test]
fn archive_unpack_clarify_does_not_bind_destination_directory_as_source_locator() {
    let root = make_temp_root("archive_unpack_missing_source_keeps_clarify");
    let destination = root.join("tmp").join("dynamic_guard_unpack_case");
    std::fs::create_dir_all(&destination).expect("destination dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.route_reason = "archive_unpack_missing_archive_locator_clarify".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveUnpack;

    let prompt = format!(
        "extract the referenced archive into {} and report the result",
        destination.display()
    );

    assert!(
        !prebind_clarify_workspace_child_locator_from_current_request(&state, &prompt, &mut route,)
    );
    assert!(!prebind_workspace_child_locator_from_resolved_prompt(
        &state, &prompt, &mut route,
    ));
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
        model_completed_workspace_file_locator_hint_should_force_clarify(
            &state,
            "Summarize the project overview in one sentence",
            &route,
            None,
            &snapshot,
        )
    );
}

#[test]
fn clarify_bare_readme_content_summary_binds_root_readme_from_current_request() {
    let root = make_temp_root("clarify_bare_readme_content_summary");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Demo\n").expect("readme");
    std::fs::write(root.join("README.zh-CN.md"), "# Demo zh\n").expect("localized readme");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.resolved_intent = "读取 README 并用恰好三句话总结。".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let prompt = "读一下 README 然后用恰好三句话总结，不要多也不要少";

    assert!(
        prebind_clarify_workspace_child_locator_from_current_request(&state, prompt, &mut route,)
    );
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
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
        "workspace_child_locator_prebound_from_clarify_current_request",
    ));
}

#[test]
fn current_project_root_name_prebinds_workspace_root_before_same_name_child() {
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
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.resolved_intent =
        "介绍 RustClaw 项目，先查证 README.md 和工作区配置文件，基于查证内容撰写三句介绍语"
            .to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;

    assert!(prebind_workspace_root_locator_from_current_request(
        &state, prompt, &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::WorkspaceProjectSummary
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(route_reason_has_marker(
        &route,
        "workspace_root_locator_prebound_from_current_request"
    ));
    let _ = std::fs::remove_dir_all(parent);
}

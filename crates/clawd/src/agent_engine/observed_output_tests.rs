use std::path::Path;

use super::super::LoopState;
use super::{
    answer_is_direct_observation_passthrough, archive_list_raw_passthrough_replacement,
    archive_list_summary_from_body, compound_listing_content_delivery_guard_entry,
    cross_turn_observed_output_entries, dir_compare_direct_answer_candidate,
    extract_direct_answer_from_generic_output, extract_direct_answer_from_generic_output_i18n,
    extract_direct_scalar_from_generic_output, extract_direct_scalar_from_generic_output_i18n,
    extract_direct_scalar_from_generic_output_with_locator_hint,
    extract_field_direct_answer_candidate, has_observed_answer_candidates,
    inventory_dir_direct_answer_candidate, multi_count_quantity_comparison_guard_entry,
    normalize_system_basic_match_path, normalized_observed_listing, observed_contract_json,
    observed_language_supports_bilingual_template, observed_output_entries,
    observed_request_language_hint, observed_request_prefers_english_template,
    observed_response_style_hint, recent_generated_output_from_user_request,
    replace_internal_missing_sentinel_with_structured_observation,
    route_allows_path_batch_scalar_path_observed_answer,
    route_disallows_direct_observation_passthrough, route_observation_facts_entry,
    route_requests_scalar_path_only, route_requires_synthesized_delivery,
    scalar_count_diagnostic_line_for_answer, scalar_route_prefers_structured_observed_answer,
    structured_observed_body, tree_summary_direct_answer_candidate, AgentRunContext,
    OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
};
use crate::executor::{StepExecutionResult, StepExecutionStatus};
use crate::{
    AppState, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
};

fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    }
}

fn error_step(step_id: &str, skill: &str, error: &str) -> StepExecutionResult {
    StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: StepExecutionStatus::Error,
        output: None,
        error: Some(error.to_string()),
        started_at: 0,
        finished_at: 0,
    }
}

#[test]
fn observed_outputs_include_structured_run_cmd_error() {
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 128",
            "platform": "linux",
            "extra": {
                "command": "git -C /tmp status",
                "exit_code": 128,
                "exit_category": "terminated_by_signal_or_shell_status",
                "stderr": "fatal: not a git repository",
                "output_truncated": false
            }
        })
    );
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(error_step("step_1", "run_cmd", &err));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(has_observed_answer_candidates(&loop_state));
    assert!(joined.contains("skill(run_cmd)"), "entries: {joined}");
    assert!(
        joined.contains("execution_status: error"),
        "entries: {joined}"
    );
    assert!(
        joined.contains("fatal: not a git repository"),
        "entries: {joined}"
    );
}

#[test]
fn observed_outputs_exclude_synthesis_steps() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"line 1"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "synthesize_answer",
        "stale synthesized answer",
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "respond", "stale delivered answer"));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"line 2"}"#,
    ));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(joined.contains("line 1"), "entries: {joined}");
    assert!(joined.contains("line 2"), "entries: {joined}");
    assert!(
        !joined.contains("stale synthesized answer"),
        "entries: {joined}"
    );
    assert!(
        !joined.contains("stale delivered answer"),
        "entries: {joined}"
    );
}

#[test]
fn multi_count_quantity_comparison_guard_lists_all_count_rows() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","path":"crates","resolved_path":"/repo/crates","recursive":false,"counts":{"total":13,"files":0,"dirs":13,"hidden":0}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"count_inventory","path":"crates/skills","resolved_path":"/repo/crates/skills","recursive":false,"counts":{"total":35,"files":0,"dirs":35,"hidden":0}}"#,
    ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;

    let guard = multi_count_quantity_comparison_guard_entry(&loop_state, Some(&route))
        .expect("multi-count guard");

    assert!(
        guard.contains("delivery_constraint=cover_all_observed_count_rows"),
        "guard: {guard}"
    );
    assert!(guard.contains("observed_count_rows=2"), "guard: {guard}");
    assert!(
        guard.contains("observed_count.1.path=/repo/crates"),
        "guard: {guard}"
    );
    assert!(
        guard.contains("observed_count.1.count_total=13"),
        "guard: {guard}"
    );
    assert!(
        guard.contains("observed_count.2.path=/repo/crates/skills"),
        "guard: {guard}"
    );
    assert!(
        guard.contains("observed_count.2.count_total=35"),
        "guard: {guard}"
    );
}

#[test]
fn compound_listing_content_delivery_guard_lists_observed_names() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["archive","release_checklist.md","service_notes.md"]}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n3|1. Verify configuration loads correctly."}"#,
    ));
    let route = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);

    let guard = compound_listing_content_delivery_guard_entry(&loop_state, Some(&route))
        .expect("compound guard");

    assert!(guard.contains("current_task_observed_listing_names"));
    assert!(guard.contains("archive, release_checklist.md, service_notes.md"));
    assert!(guard.contains("current_task_observed_content_excerpt: present"));
}

#[test]
fn observed_outputs_keep_latest_content_read_for_same_path() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","resolved_path":"/tmp/model_io.log","excerpt":"old head evidence"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","resolved_path":"/tmp/model_io.log","excerpt":"new tail evidence"}"#,
    ));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(!joined.contains("old head evidence"), "entries: {joined}");
    assert!(joined.contains("new tail evidence"), "entries: {joined}");
}

fn chat_wrapped_unclassified_route(response_shape: OutputResponseShape) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Run an observation, then produce the requested final wording."
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/workspace/project".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

#[test]
fn scalar_path_observed_route_rejects_content_evidence_contract() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.requires_content_evidence = true;

    assert!(route_requests_scalar_path_only(&route));
    assert!(!route_allows_path_batch_scalar_path_observed_answer(&route));

    route.output_contract.requires_content_evidence = false;
    assert!(route_allows_path_batch_scalar_path_observed_answer(&route));
}

#[test]
fn scalar_count_answer_detects_non_numeric_diagnostic_line() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config_copy".to_string();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "0\n\nfind: /workspace/configs/config_copy: No such file or directory\n",
    ));

    let diagnostic = scalar_count_diagnostic_line_for_answer("0", Some(&route), &loop_state);

    assert_eq!(
        diagnostic.as_deref(),
        Some("find: /workspace/configs/config_copy: No such file or directory")
    );
}

fn reuse_active_context(user_request: &str) -> AgentRunContext {
    AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskAppend),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        user_request: Some(user_request.to_string()),
        ..Default::default()
    }
}

#[test]
fn recent_generated_output_extracts_internal_merge_block() {
    let merged = "Current task:\nlook at that docs dir\n\nMost recent generated output:\narchive\nrelease_checklist.md\nservice_notes.md\n\nContinuity rules:\n- keep scope\n\nNew user instruction:\ncount only";

    assert_eq!(
        recent_generated_output_from_user_request(merged).as_deref(),
        Some("archive\nrelease_checklist.md\nservice_notes.md")
    );
}

#[test]
fn cross_turn_observed_entries_require_reuse_active_context() {
    let merged = "Current task:\nlook at that docs dir\n\nMost recent generated output:\narchive\nrelease_checklist.md\nservice_notes.md\n\nContinuity rules:\n- keep scope";
    let loop_state = LoopState::new(1);
    let allowed = reuse_active_context(merged);

    let entries = cross_turn_observed_output_entries(&loop_state, Some(&allowed));
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("prior_turn_observed_output"));
    assert!(entries[0].contains("archive"));
    assert!(!entries[0].contains("Continuity rules"));

    let standalone = AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        user_request: Some(merged.to_string()),
        ..Default::default()
    };
    assert!(cross_turn_observed_output_entries(&loop_state, Some(&standalone)).is_empty());
}

#[test]
fn direct_scalar_ignores_exit_zero_prefix() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "git_basic", "exit=0\nmain\n"));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("main")
    );
}

#[test]
fn direct_scalar_extracts_system_basic_runtime_status_value() {
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route_result.output_contract.locator_kind = OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..Default::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"runtime_status","kind":"current_user","value":"guagua","field_value":"guagua","command_output":"guagua"}"#,
    ));

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("guagua")
    );
}

#[test]
fn direct_scalar_defers_git_oneline_log_record_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n09342a6a fix: expose nl execution and locator flows\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "查看当前工作区最近一次 git 提交的标题，并简短告诉我。".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "Self-contained workspace inspection request for git commit title."
            .to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn observed_entries_include_structured_extract_field_outputs() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#,
        ));

    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 2);
    assert!(entries[0].contains("name: react-example"));
    assert!(entries[1].contains("package.name: clawd"));
}

#[test]
fn direct_scalar_ignores_shell_locale_warning_noise() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "/tmp/rustclaw-workspace\n\nbash: warning: setlocale: LC_ALL: cannot change locale (C.UTF-8): No such file or directory\n",
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("/tmp/rustclaw-workspace")
    );
}

#[test]
fn direct_scalar_reads_extract_field_value_from_structured_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("rustclaw")
    );
}

#[test]
fn direct_scalar_reads_read_field_value_from_structured_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","exists":true,"field_path":"package.name","value_text":"react-example","value":"react-example","value_type":"string"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("react-example")
    );
}

#[test]
fn direct_scalar_defers_container_read_field_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","exists":true,"field_path":"scripts","value":{"build":"echo build","dev":"echo dev"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\"}","value_type":"object"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None),
        None
    );
}

#[test]
fn direct_scalar_returns_container_read_field_json_for_scalar_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","exists":true,"field_path":"package.version","value":{"workspace":true},"value_text":"{\"workspace\":true}","value_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "Cargo.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(r#"{"workspace":true}"#)
    );
}

#[test]
fn direct_scalar_preserves_resolved_extract_field_label_for_non_exact_match() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"model.vendor","resolved_field_path":"llm.selected_vendor","match_strategy":"missing_parent_leaf_key_suffix","value_text":"minimax","value":"minimax","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/config.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("llm.selected_vendor: minimax")
    );
}

#[test]
fn direct_scalar_reads_array_identity_field_value_without_label() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"archive_basic.group","resolved_field_path":"skills[name=archive_basic].group","match_strategy":"array_item_key_path","value_text":"system","value":"system","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/skills_registry.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("system")
    );
}

#[test]
fn direct_answer_reads_array_identity_extract_field_value_without_label() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"skills.[name=archive_basic].group","resolved_field_path":"skills.[name=archive_basic].group","match_strategy":"exact_path","value_text":"system","value":"system","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/skills_registry.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("system")
    );
}

#[test]
fn direct_answer_reads_config_basic_extract_field_value() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"run_cmd.planner_kind","value_text":"tool","value":"tool","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/skills_registry.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("run_cmd.planner_kind: tool")
    );
    assert!(has_observed_answer_candidates(&loop_state));
}

#[test]
fn direct_answer_reads_config_basic_read_fields_values() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_fields","path":"package.json","resolved_path":"/tmp/package.json","count":2,"results":[{"field_path":"name","exists":true,"value_type":"string","value_text":"react-example","value":"react-example"},{"field_path":"version","exists":true,"value_type":"string","value_text":"1.0.0","value":"1.0.0"}]}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "package.json".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("name: react-example\nversion: 1.0.0")
    );
}

#[test]
fn direct_scalar_reads_structured_keys_value_list() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"structured_keys","exists":true,"container_type":"object","count":3,"keys":["app","features","paths"],"field_path":""}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/configs/app_config.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("app\nfeatures\npaths")
    );
}

#[test]
fn direct_answer_does_not_treat_root_level_as_missing_key() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"structured_keys","exists":true,"container_type":"object","count":3,"keys":["app","features","paths"],"field_path":""}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.resolved_intent = "List root-level keys in app_config.toml only".to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "app_config.toml".to_string();
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "List root-level keys in scripts/nl_tests/fixtures/device_local/configs/app_config.toml only."
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("app\nfeatures\npaths")
    );
}

#[test]
fn direct_answer_defers_container_extract_field_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "package.json".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_formats_schema_enum_extract_field_with_resolved_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"target","resolved_field_path":"properties.reference_resolution.properties.target","match_strategy":"unique_bare_key","value":{"type":"string","enum":["none","current_action_result","current_turn_locator"]},"value_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "prompts/schemas/direct_answer_gate.schema.json".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("schema enum should be formatted without synthesis");

    assert!(answer.contains("properties.reference_resolution.properties.target"));
    assert!(answer.contains("`none`"));
    assert!(answer.contains("`current_turn_locator`"));
}

#[test]
fn direct_answer_formats_config_basic_validate_result_as_pass_fail() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"validate_structured","path":"configs/config.toml","resolved_path":"/tmp/configs/config.toml","format":"toml","valid":true,"root_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/config.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        original_user_request: Some(
            "Validate configs/config.toml and answer pass or fail.".to_string(),
        ),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("pass: toml parsed successfully")
    );
}

#[test]
fn direct_scalar_formats_config_validation_result_in_request_language() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"validate_structured","path":"configs/config.toml","resolved_path":"/tmp/configs/config.toml","format":"toml","valid":true,"root_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/config.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        original_user_request: Some("只检查 configs/config.toml 是否是合法 TOML。".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("通过：toml 解析成功")
    );
}

#[test]
fn direct_scalar_defers_recent_structured_scalar_comparison_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_fields","path":"UI/package.json","resolved_path":"/tmp/UI/package.json","count":1,"results":[{"field_path":"name","exists":true,"value_type":"string","value_text":"react-example","value":"react-example"}]}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","path":"crates/clawd/Cargo.toml","resolved_path":"/tmp/crates/clawd/Cargo.toml","field_path":"package.name","exists":true,"value_type":"string","value_text":"clawd","value":"clawd"}"#,
        ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "UI/package.json 里的 name 和 crates/clawd/Cargo.toml 里的 package.name 一样吗？只回答一样或不一样"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:compare_targets".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::QuantityComparison,
                locator_hint: "UI/package.json|crates/clawd/Cargo.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_scalar_formats_recent_structured_scalar_equality() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","field_path":"name","exists":true,"value_text":"RustClaw","value":"RustClaw","value_type":"string"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","field_path":"crate_name","exists":true,"value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Are those two names the same? Answer same or different".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:same_or_different".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("RustClaw and rustclaw are different.")
    );
}

#[test]
fn direct_scalar_equality_ignores_duplicate_structured_source() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","path":"/tmp/Cargo.toml","resolved_path":"/tmp/Cargo.toml","field_path":"workspace.package.version","resolved_field_path":"workspace.package.version","exists":true,"value_text":"0.1.7","value":"0.1.7","value_type":"string"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "config_basic",
            r#"{"action":"extract_field","path":"/tmp/Cargo.toml","resolved_path":"/tmp/Cargo.toml","field_path":"workspace.package.version","resolved_field_path":"workspace.package.version","exists":true,"value_text":"0.1.7","value":"0.1.7","value_type":"string"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Compare the Cargo.toml version with the version mentioned in README.md."
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:compare_targets".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: Some(1),
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
            locator_hint: "/tmp/Cargo.toml | /tmp/README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_formats_recent_structured_scalar_equality_for_strict_route() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","field_path":"name","exists":true,"value_text":"rustclaw-nl-fixture","value":"rustclaw-nl-fixture","value_type":"string"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "config_basic",
            r#"{"action":"extract_field","field_path":"package.name","exists":true,"value_text":"clawd","value":"clawd","value_type":"string"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "Read two names and answer in one line with whether they are the same or different."
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:same_or_different".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: Some(1),
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
            locator_hint:
                "scripts/nl_tests/fixtures/device_local/package.json and crates/clawd/Cargo.toml"
                    .to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some("Read the name from scripts/nl_tests/fixtures/device_local/package.json. Read package.name from crates/clawd/Cargo.toml. Then answer in one line with the two names and whether they are the same or different.".to_string()),
            ..AgentRunContext::default()
        };
    assert_eq!(
        extract_direct_answer_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("rustclaw-nl-fixture and clawd are different.")
    );
}

#[test]
fn structured_pair_answer_does_not_infer_fields_from_read_file_outputs() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "read_file",
        r#"{"name":"react-example","version":"0.0.0"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "read_file",
        r#"[package]
name = "clawd"
version.workspace = true
"#,
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "读取 UI/package.json 里的 name 字段，再读取 crates/clawd/Cargo.toml 里的 package.name 字段，最后用一行输出：前者、后者、一样或不一样"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:same_or_different".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let _agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        super::recent_structured_scalar_observation_count(&loop_state),
        0
    );
}

#[test]
fn direct_scalar_reports_missing_extract_field_as_readable_message() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("未找到 name 字段")
    );
}

#[test]
fn internal_missing_sentinel_uses_structured_extract_field_evidence() {
    let state = AppState::test_default_with_fixture_provider();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"package.name","value_text":"","value":null,"value_type":"null"}"#,
        ));

    assert_eq!(
        replace_internal_missing_sentinel_with_structured_observation(
            "<missing>",
            &state,
            &loop_state,
            None
        )
        .as_deref(),
        Some("未找到 package.name 字段")
    );
    assert_eq!(
        replace_internal_missing_sentinel_with_structured_observation(
            "package.name: <missing>",
            &state,
            &loop_state,
            None
        )
        .as_deref(),
        Some("未找到 package.name 字段")
    );
}

#[test]
fn direct_scalar_missing_field_language_uses_original_request_before_resolved_prompt() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Read the name field from package.json and output only its value."
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:field_extract".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        original_user_request: Some("读取 package.json 里的 name 字段，只输出值".to_string()),
        user_request: Some(
            "Read the name field from package.json and output only its value.".to_string(),
        ),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context),
        )
        .as_deref(),
        Some("未找到 name 字段")
    );
}

#[test]
fn direct_scalar_defers_count_inventory_total_with_component_breakdown_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"count_inventory","counts":{"total":12,"files":9,"dirs":3}}"#,
    ));
    assert!(extract_direct_scalar_from_generic_output(&loop_state, None).is_none());
}

#[test]
fn direct_scalar_reads_count_inventory_single_dimension_from_structured_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","kind_filter":"file","counts":{"total":12,"files":9,"dirs":3}}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("9")
    );
}

#[test]
fn direct_count_inventory_uses_total_when_response_contract_is_known() {
    let value = serde_json::json!({
        "action": "count_inventory",
        "counts": {"total": 66, "files": 40, "dirs": 26},
        "path": ".",
        "recursive": false
    });

    assert!(super::count_inventory_direct_answer_candidate(None, &value, None, false,).is_none());

    assert_eq!(
        super::count_inventory_direct_answer_candidate(
            None,
            &value,
            Some(OutputResponseShape::Scalar),
            false,
        )
        .as_deref(),
        Some("66")
    );

    let one_sentence = super::count_inventory_direct_answer_candidate(
        None,
        &value,
        Some(OutputResponseShape::OneSentence),
        false,
    )
    .expect("one-sentence count answer");
    assert!(one_sentence.contains("66"));
}

#[test]
fn inventory_dir_grouped_contract_uses_names_by_kind() {
    let value = serde_json::json!({
        "action": "inventory_dir",
        "names_only": true,
        "names": ["Cargo.toml", "src", "README.md"],
        "names_by_kind": {
            "files": ["Cargo.toml", "README.md"],
            "dirs": ["src"],
            "other": []
        },
        "counts": {"files": 2, "dirs": 1, "total": 3}
    });
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;

    let answer = inventory_dir_direct_answer_candidate(None, Some(&route), &value, false)
        .expect("grouped inventory answer");

    assert!(answer.contains("目录:"));
    assert!(answer.contains("- src"));
    assert!(answer.contains("文件:"));
    assert!(answer.contains("- Cargo.toml"));
    assert!(answer.contains("- README.md"));
}

#[test]
fn inventory_dir_file_names_contract_filters_names_by_kind() {
    let value = serde_json::json!({
        "action": "inventory_dir",
        "names_only": true,
        "names": ["archive", "release_checklist.md", "service_notes.md"],
        "names_by_kind": {
            "files": ["release_checklist.md", "service_notes.md"],
            "dirs": ["archive"],
            "other": []
        },
        "counts": {"files": 2, "dirs": 1, "total": 3}
    });
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::FileNames;

    let answer = inventory_dir_direct_answer_candidate(None, Some(&route), &value, false)
        .expect("file names answer");

    assert!(answer.contains("release_checklist.md"));
    assert!(answer.contains("service_notes.md"));
    assert!(!answer.contains("archive"));
}

#[test]
fn direct_answer_groups_inventory_dir_for_chat_wrapped_directory_entry_contract() {
    let mut loop_state = LoopState::new(1);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/root","names_only":false,"names":["docs","README.md"],"names_by_kind":{"files":["README.md"],"dirs":["docs"],"other":[]},"counts":{"files":1,"dirs":1,"total":2}}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::DirectoryEntryGroups;
    let context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&context))
        .expect("inventory_dir should produce grouped direct answer");

    assert!(answer.contains("目录:") || answer.contains("Directories:"));
    assert!(answer.contains("- docs"));
    assert!(answer.contains("文件:") || answer.contains("Files:"));
    assert!(answer.contains("- README.md"));
}

#[test]
fn tree_summary_direct_answer_lists_top_level_groups_without_false_truncation() {
    let value = serde_json::json!({
        "action": "tree_summary",
        "path": "/tmp/root",
        "resolved_path": "/tmp/root",
        "truncated_nodes": 0,
        "tree": {
            "kind": "dir",
            "path": "/tmp/root",
            "child_count": 3,
            "omitted_children": 0,
            "children": [
                {
                    "kind": "dir",
                    "path": "/tmp/root/configs",
                    "child_count": 1,
                    "omitted_children": 0,
                    "children": []
                },
                {
                    "kind": "file",
                    "path": "/tmp/root/package.json",
                    "size_bytes": 10
                },
                {
                    "kind": "dir",
                    "path": "/tmp/root/logs",
                    "child_count": 1,
                    "omitted_children": 0,
                    "children": []
                }
            ]
        }
    });

    let answer = tree_summary_direct_answer_candidate(None, &value, false).expect("answer");

    assert!(answer.contains("顶层结构"), "answer: {answer}");
    assert!(answer.contains("configs/"), "answer: {answer}");
    assert!(answer.contains("logs/"), "answer: {answer}");
    assert!(answer.contains("package.json"), "answer: {answer}");
    assert!(!answer.contains("未显示"), "answer: {answer}");
    assert!(!answer.contains("截断"), "answer: {answer}");
}

#[test]
fn dir_compare_direct_answer_reports_no_differences() {
    let value = serde_json::json!({
        "action": "dir_compare",
        "left_path": "tmp/bundle_src",
        "right_path": "tmp/dynamic_guard_unpack_case",
        "counts": {
            "left_only": 0,
            "right_only": 0,
            "kind_mismatches": 0,
            "common": 3
        }
    });

    let answer = dir_compare_direct_answer_candidate(None, &value, true).expect("answer");

    assert_eq!(answer, "No differences found.");
}

#[test]
fn direct_count_inventory_answer_uses_file_count_and_explanation_for_one_sentence() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":53,"files":53,"dirs":0},"kind_filter":"file","path":".","recursive":false}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "数一下当前目录一级有多少个普通文件，只告诉我数字和一句解释".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "scalar_count".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarCount,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        original_user_request: Some(
            "数一下当前目录一级有多少个普通文件，只告诉我数字和一句解释".to_string(),
        ),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("count_inventory should produce a direct count answer");

    assert!(answer.contains("53"));
    assert!(answer.contains("普通文件"));
    assert!(!answer.contains("无法计数"));
}

#[test]
fn direct_scalar_prefers_unique_exact_fs_search_match_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"README.md","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("README.md")
    );
}

#[test]
fn direct_scalar_uses_locator_hint_when_fs_search_output_omits_pattern() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output_with_locator_hint(
            &loop_state,
            Some("README.md"),
            None,
            false,
        )
        .as_deref(),
        Some("README.md")
    );
}

#[test]
fn direct_scalar_does_not_collapse_ambiguous_fs_search_to_count() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"README","count":2,"results":["README.md","README.txt"],"root":""}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None),
        None
    );
}

#[test]
fn direct_scalar_prefers_locator_extension_when_fs_search_pattern_is_broad() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"execution_intent","count":2,"results":["plan/execution_intent_route_trace_cases.txt","plan/execution_intent_routing_repair_plan_20260509.md"],"root":"plan"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output_with_locator_hint(
            &loop_state,
            Some("plan/extra_missing_repair_probe.md"),
            None,
            false,
        )
        .as_deref(),
        Some("plan/execution_intent_routing_repair_plan_20260509.md")
    );
}

#[test]
fn fs_search_file_paths_contract_filters_with_structured_pattern() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","pattern":"execution_intent","count":8,"results":["crates/clawd/src/agent_engine/planning.rs","docs/planning_deterministic_guardrails_audit.md","plan/agent_intelligence_architecture_plan_20260511_已完成.md","plan/builtin_skill_capability_governance_plan_20260510.md","plan/codex_style_agent_architecture_refactor_plan_20260511.md","plan/execution_intent_routing_repair_plan_20260509_已完成.md","plan/llm_first_agent_convergence_plan_20260511.md","prompts/layers/overlays/plan_repair_prompt.md"],"root":""}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent = "Read plan/definitely_missing_20260511.md; if missing, search the plan directory for md files related to execution_intent and return only found paths.".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/plan".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("plan/execution_intent_routing_repair_plan_20260509_已完成.md")
    );
}

#[test]
fn fs_search_file_paths_contract_preserves_multi_candidates_when_not_decisive() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"find_name","pattern":"README","count":5,"results":["README.md","README.zh-CN.md","UI/README.md","data/vendor/whisper.cpp/examples/whisper.android.java/README_files","data/vendor/whisper.cpp/README.md"],"root":""}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent =
            "Find files named README under the current repo. If there are multiple candidates, list candidates instead of choosing one."
                .to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/home/guagua/rustclaw".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("multi-candidate search should produce a direct candidate list");

    assert!(answer.contains("README.md"));
    assert!(answer.contains("README.zh-CN.md"));
    assert!(
        answer.contains('\n'),
        "answer should not collapse to one path: {answer}"
    );
    assert_ne!(
        answer.trim(),
        "data/vendor/whisper.cpp/examples/whisper.android.java/README_files"
    );
}

#[test]
fn fs_search_file_paths_contract_i18n_expands_to_five_full_paths() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"find_name","count":6,"results":["README.md","README.zh-CN.md","README_cn.md","RUSTCLAW_SERVICE_README.md","UI/README.md","Cargo.toml"],"root":""}"#,
    ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent =
        "Find README-like files in the current repository and list the first five full paths."
            .to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = String::new();
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let state = AppState::test_default_with_fixture_provider();
    let answer = extract_direct_answer_from_generic_output_i18n(
        &loop_state,
        &state,
        Some(&agent_run_context),
    )
    .expect("file_paths should produce a full path list");
    let lines = answer.lines().collect::<Vec<_>>();
    let root = state.skill_rt.workspace_root.display().to_string();

    assert_eq!(lines.len(), 5, "answer: {answer}");
    assert!(lines.iter().all(|line| line.starts_with(&root)));
    assert!(answer.contains("/README.md"));
    assert!(answer.contains("/UI/README.md"));
    assert!(!answer.contains("Cargo.toml"));
}

#[test]
fn direct_scalar_count_uses_latest_fs_search_count() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"count_inventory","counts":{"total":107,"files":101,"dirs":6},"path":"scripts/nl_tests/cases"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "fs_search",
            r#"{"action":"find_name","count":10,"patterns":["clarify"],"results":["a.txt","b.txt"],"root":"scripts/nl_tests/cases"}"#,
        ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("10")
    );
}

#[test]
fn fs_search_find_ext_direct_answer_returns_paths_list() {
    let value = serde_json::json!({
        "action": "find_ext",
        "ext": "toml",
        "count": 3,
        "results": ["Cargo.toml", "configs/config.toml", "configs/git_basic.toml"]
    });
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, false).as_deref(),
        Some("Cargo.toml\nconfigs/config.toml\nconfigs/git_basic.toml")
    );
}

#[test]
fn fs_search_grep_text_direct_answer_returns_unique_matching_paths() {
    let value = serde_json::json!({
        "action": "grep_text",
        "query": "FirstLayerDecision",
        "count": 1,
        "match_count": 2,
        "matches": [
            {"path": "README.md", "line": 45, "text": "FirstLayerDecision"},
            {"path": "README.md", "line": 95, "text": "FirstLayerDecision"}
        ]
    });

    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, false, false)
            .as_deref(),
        Some("README.md")
    );
}

#[test]
fn fs_search_grep_text_direct_answer_preserves_path_answer_when_requested() {
    let value = serde_json::json!({
        "action": "grep_text",
        "query": "FirstLayerDecision",
        "count": 4,
        "match_count": 5,
        "matches": [
            {"path": "README.md", "line": 45, "text": "FirstLayerDecision"},
            {"path": "README.md", "line": 95, "text": "FirstLayerDecision"},
            {"path": "crates/clawd/src/ask_flow.rs", "line": 10, "text": "FirstLayerDecision"},
            {"path": "crates/clawd/src/intent_router.rs", "line": 20, "text": "FirstLayerDecision"},
            {"path": "crates/clawd/src/main.rs", "line": 30, "text": "FirstLayerDecision"}
        ]
    });

    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, true).as_deref(),
        Some("README.md\ncrates/clawd/src/ask_flow.rs\ncrates/clawd/src/intent_router.rs")
    );
}

#[test]
fn fs_search_grep_text_direct_answer_returns_matching_lines_when_listing_allowed() {
    let value = serde_json::json!({
        "action": "grep_text",
        "query": "ERROR",
        "count": 1,
        "match_count": 1,
        "matches": [
            {
                "path": "logs/app.log",
                "line": 16,
                "text": "2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata"
            }
        ]
    });

    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, false).as_deref(),
        Some("16: 2026-04-01 10:08:44 ERROR provider timeout while fetching external metadata")
    );
}

#[test]
fn fs_search_grep_text_direct_answer_uses_name_matches_when_content_empty() {
    let value = serde_json::json!({
        "action": "grep_text",
        "query": "abcd",
        "count": 0,
        "match_count": 0,
        "matches": [],
        "name_count": 4,
        "name_results": [
            "abcd_report.md",
            "my_abcd.txt",
            "x_abcd_log.txt",
            "zz_abcd_backup.log"
        ]
    });

    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, false).as_deref(),
        Some("abcd_report.md\nmy_abcd.txt\nx_abcd_log.txt")
    );
}

#[test]
fn virtual_fs_basic_grep_text_output_can_direct_answer_file_paths() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"grep_text","query":"FirstLayerDecision","count":4,"match_count":5,"matches":[{"path":"README.md","line":45,"text":"FirstLayerDecision"},{"path":"README.md","line":95,"text":"FirstLayerDecision"},{"path":"crates/clawd/src/ask_flow.rs","line":10,"text":"FirstLayerDecision"},{"path":"crates/clawd/src/intent_router.rs","line":20,"text":"FirstLayerDecision"},{"path":"crates/clawd/src/main.rs","line":30,"text":"FirstLayerDecision"}]}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::FilePaths;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("README.md\ncrates/clawd/src/ask_flow.rs\ncrates/clawd/src/intent_router.rs")
    );
}

#[test]
fn content_presence_direct_answer_includes_matching_text_evidence() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r##"{"action":"grep_text","query":"release","case_insensitive":true,"count":1,"match_count":1,"matches":[{"path":"scripts/nl_tests/fixtures/device_local/docs/release_checklist.md","line":1,"text":"# Release Checklist"}]}"##,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("content presence should produce a direct grounded answer");

    assert!(answer.contains("release"));
    assert!(answer.contains("release_checklist.md:1"));
    assert!(answer.contains("# Release Checklist"));
}

#[test]
fn content_presence_direct_answer_uses_name_results_when_content_empty() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r##"{"action":"grep_text","query":"abcd","case_insensitive":false,"count":0,"match_count":0,"matches":[],"name_count":4,"name_results":["scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"],"root":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"}"##,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("name matches should produce a grounded direct answer");

    assert_eq!(
        answer,
        "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log"
    );
}

#[test]
fn doc_parse_content_presence_uses_machine_selector_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "doc_parse",
        r##"{"text":"# Release Checklist\n\n1. Verify config loading.\n2. Confirm migrations."}"##,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentPresenceCheck;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string();
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            auto_locator_path: Some(
                "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string(),
            ),
            original_user_request: Some(
                "[CONTRACT_TEST_HINT]\nselector_query=release\nselector_case_insensitive=true\n[/CONTRACT_TEST_HINT]"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("doc_parse content presence should use selector evidence directly");

    assert!(answer.contains("release"));
    assert!(answer.contains("release_checklist.md:1"));
    assert!(answer.contains("# Release Checklist"));
    assert!(!answer.contains("不包含"));
}

#[test]
fn fs_search_grep_text_observed_body_keeps_line_evidence() {
    let body = r#"{"action":"grep_text","query":"run_cmd","patterns":["prompt_utils.rs"],"count":1,"match_count":2,"matches":[{"path":"crates/clawd/src/prompt_utils.rs","line":1275,"text":"if step_type == \"run_cmd\" {"},{"path":"crates/clawd/src/prompt_utils.rs","line":1276,"text":"return normalize_run_cmd_call(obj, obj.get(\"args\").and_then(|v| v.as_object()));"}]}"#;
    let observed = super::structured_observed_body("fs_search", body)
        .expect("grep_text should compact observed evidence");

    assert!(observed.contains("grep_text query=run_cmd"));
    assert!(observed.contains("file_patterns=prompt_utils.rs"));
    assert!(observed.contains("match path=crates/clawd/src/prompt_utils.rs line=1275"));
    assert!(observed.contains("step_type == \"run_cmd\""));
}

#[test]
fn fs_search_grep_text_observed_body_keeps_name_match_fallback() {
    let body = r#"{"action":"grep_text","query":"abcd","count":0,"match_count":0,"matches":[],"name_count":1,"name_results":["my_abcd.txt"]}"#;
    let observed = super::structured_observed_body("fs_search", body)
        .expect("grep_text should compact name fallback evidence");

    assert!(observed.contains("grep_text query=abcd"));
    assert!(observed.contains("name_count=1"));
    assert!(observed.contains("name_match path=my_abcd.txt"));
    assert!(observed.contains("matches: none"));
}

#[test]
fn fs_search_find_ext_directory_contract_returns_parent_dirs() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_ext","ext":"sh","count":4,"results":["system_report.sh","scripts/run.sh","scripts/dev/check.sh","component_start/start-clawd.sh"],"root":""}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "list directories containing sh files".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::DirectoryNames,
            ..IntentOutputContract::default()
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(".\nscripts\nscripts/dev\ncomponent_start")
    );
}

#[test]
fn virtual_fs_basic_find_ext_directory_contract_returns_parent_dirs() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"find_ext","ext":"sh","count":4,"results":["system_report.sh","scripts/run.sh","scripts/dev/check.sh","component_start/start-clawd.sh"],"root":""}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "list unique directories containing sh scripts".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::DirectoryNames,
            ..IntentOutputContract::default()
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(".\nscripts\nscripts/dev\ncomponent_start")
    );
}

#[test]
fn directory_purpose_summary_find_ext_direct_answer_keeps_full_candidate_list() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"find_ext","count":5,"ext":"toml","results":["Cargo.toml","configs/config.toml","configs/skills_registry.toml","configs/channels/telegram.toml","configs/i18n/rss_fetch.en-US.toml"],"root":""},"text":"{\"action\":\"find_ext\",\"count\":5,\"ext\":\"toml\",\"results\":[\"Cargo.toml\",\"configs/config.toml\",\"configs/skills_registry.toml\",\"configs/channels/telegram.toml\",\"configs/i18n/rss_fetch.en-US.toml\"],\"root\":\"\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "config_basic",
        r#"{"action":"extract_fields","path":"Cargo.toml","count":1,"results":[{"field_path":"workspace.dependencies.toml","exists":true,"value_text":"0.8"}]}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    route_result.resolved_intent =
        "Find all TOML files in the repository and briefly describe representative entries"
            .to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::DirectoryPurposeSummary;
    route_result.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("find_ext directory summary should produce a direct answer");

    assert!(answer.contains("find_ext.ext=toml"), "{answer}");
    assert!(answer.contains("find_ext.count=5"), "{answer}");
    assert!(answer.contains("Cargo.toml"), "{answer}");
    assert!(answer.contains("configs/config.toml"), "{answer}");
    assert!(answer.contains("configs/skills_registry.toml"), "{answer}");
    assert!(
        answer.contains("configs/channels/telegram.toml"),
        "{answer}"
    );
    assert!(
        answer.contains("configs/i18n/rss_fetch.en-US.toml"),
        "{answer}"
    );
    assert!(
        answer.contains("find_ext.representative.path=Cargo.toml; kind=root"),
        "{answer}"
    );
    assert!(
        answer.contains("find_ext.representative.path=configs/config.toml; kind=config"),
        "{answer}"
    );
    assert!(
        !answer.trim().starts_with("workspace.dependencies.toml"),
        "{answer}"
    );
}

#[test]
fn multi_status_json_direct_answer_keeps_all_observed_status_files() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|{\"kind\":\"telegram\",\"name\":\"primary\",\"scope\":\"telegram:primary\",\"healthy\":true,\"status\":\"running\",\"last_error\":null}","path":"/home/guagua/rustclaw/run/gateway-instance-status/telegram__primary.json","resolved_path":"/home/guagua/rustclaw/run/gateway-instance-status/telegram__primary.json"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|{\\\"kind\\\":\\\"telegram\\\",\\\"name\\\":\\\"primary\\\",\\\"scope\\\":\\\"telegram:primary\\\",\\\"healthy\\\":true,\\\"status\\\":\\\"running\\\",\\\"last_error\\\":null}\",\"path\":\"/home/guagua/rustclaw/run/gateway-instance-status/telegram__primary.json\",\"resolved_path\":\"/home/guagua/rustclaw/run/gateway-instance-status/telegram__primary.json\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|{\"name\":\"primary\",\"healthy\":true,\"status\":\"running\",\"last_error\":null}","path":"/home/guagua/rustclaw/run/telegram-bot-status/primary.json","resolved_path":"/home/guagua/rustclaw/run/telegram-bot-status/primary.json"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|{\\\"name\\\":\\\"primary\\\",\\\"healthy\\\":true,\\\"status\\\":\\\"running\\\",\\\"last_error\\\":null}\",\"path\":\"/home/guagua/rustclaw/run/telegram-bot-status/primary.json\",\"resolved_path\":\"/home/guagua/rustclaw/run/telegram-bot-status/primary.json\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"read_range","excerpt":"1|{\n2|  \"healthy\": true,\n3|  \"status\": \"login_required\",\n4|  \"last_error\": null,\n5|  \"account_label\": \"primary\"\n6|}","path":"/home/guagua/rustclaw/run/wechatd-status/primary.json","resolved_path":"/home/guagua/rustclaw/run/wechatd-status/primary.json"},"text":"{\"action\":\"read_range\",\"excerpt\":\"1|{\\n2|  \\\"healthy\\\": true,\\n3|  \\\"status\\\": \\\"login_required\\\",\\n4|  \\\"last_error\\\": null,\\n5|  \\\"account_label\\\": \\\"primary\\\"\\n6|}\",\"path\":\"/home/guagua/rustclaw/run/wechatd-status/primary.json\",\"resolved_path\":\"/home/guagua/rustclaw/run/wechatd-status/primary.json\"}"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "synthesize_answer",
        r#"{"healthy":true,"status":"login_required","account_label":"primary"}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    route_result.resolved_intent =
        "run a basic health check here and summarize only the most important findings".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/home/guagua/rustclaw/run".to_string()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("multi status observations should produce a combined direct answer");

    assert!(answer.contains("status_files.count=3"), "{answer}");
    assert!(
        answer.contains("gateway-instance-status/telegram__primary.json"),
        "{answer}"
    );
    assert!(
        answer.contains("telegram-bot-status/primary.json"),
        "{answer}"
    );
    assert!(answer.contains("wechatd-status/primary.json"), "{answer}");
    assert!(answer.contains("status=running"), "{answer}");
    assert!(answer.contains("status=login_required"), "{answer}");
    assert!(
        answer.contains("status_files.notable.status=login_required"),
        "{answer}"
    );
}

#[test]
fn fs_search_direct_answer_does_not_confirm_ambiguous_matches_when_direct_list_disallowed() {
    let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"abcd","count":4,"results":["abcd_report.md","my_abcd.txt","x_abcd_log.txt","zz_abcd_backup.log"],"root":""}"#,
        )
        .expect("json");
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, false, false)
            .as_deref(),
        None
    );
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, true, false).as_deref(),
        Some("abcd_report.md\nmy_abcd.txt\nx_abcd_log.txt")
    );
}

#[test]
fn fs_search_direct_answer_prefers_exact_match_before_confirmation() {
    let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","pattern":"README.md","count":5,"results":["RUSTCLAW_SERVICE_README.md","UI/README.md","README.md","pi_app/README.md","skill_develop/README.md"],"root":""}"#,
        )
        .expect("json");
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, false, false)
            .as_deref(),
        Some("有，路径：README.md")
    );
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, None, false, false, true).as_deref(),
        Some("README.md")
    );
}

#[test]
fn direct_answer_for_strict_file_names_fs_search_uses_plain_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":1,"results":["scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt"],"root":"scripts/nl_tests/fixtures/locator_smart/stem_unique"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "在目标目录里找 abcd，只输出路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "scripts/nl_tests/fixtures/locator_smart/stem_unique".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("scripts/nl_tests/fixtures/locator_smart/stem_unique/ABCD.txt")
    );
}

#[test]
fn fs_search_direct_answer_uses_locator_hint_for_ambiguous_list_when_allowed() {
    let value = serde_json::from_str::<serde_json::Value>(
            r#"{"action":"find_name","count":4,"results":["scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/zz_abcd_backup.log","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt","scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt"],"root":"scripts/nl_tests/fixtures/locator_smart/fuzzy_top3"}"#,
        )
        .expect("json");
    assert_eq!(
        super::fs_search_direct_answer_candidate(None, &value, Some("abcd"), false, false, false)
            .as_deref(),
        None
    );
    assert_eq!(
            super::fs_search_direct_answer_candidate(
                None,
                &value,
                Some("abcd"),
                false,
                true,
                false
            )
            .as_deref(),
            Some(
                "scripts/nl_tests/fixtures/locator_smart/fuzzy_top3/abcd_report.md\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/my_abcd.txt\nscripts/nl_tests/fixtures/locator_smart/fuzzy_top3/x_abcd_log.txt"
            )
        );
}

#[test]
fn observed_entries_keep_latest_listing_plus_recent_non_listing_steps() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a.md\nb.md\nc.md\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "read_file", "# A\nalpha\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "read_file", "# B\nbeta\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_4", "read_file", "# C\ngamma\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_5", "read_file", "# D\ndelta\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_6", "read_file", "# E\nepsilon\n"));

    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 5);
    assert!(entries
        .iter()
        .any(|entry| entry.contains("step_1 skill(list_dir)")));
    assert!(entries
        .iter()
        .any(|entry| entry.contains("step_6 skill(read_file)")));
    assert!(!entries
        .iter()
        .any(|entry| entry.contains("step_2 skill(read_file)")));
}

#[test]
fn normalized_listing_trims_blank_lines() {
    assert_eq!(
        normalized_observed_listing("\nfoo\n\nbar\n").as_deref(),
        Some("foo\nbar")
    );
}

#[test]
fn observed_entries_use_read_range_excerpt_body_instead_of_raw_json() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"read_range","path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|Hello"}"#,
    ));
    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("read_range path=/tmp/README.md"));
    assert!(entries[0].contains("# RustClaw"));
    assert!(entries[0].contains("# RustClaw\n\nHello"));
    assert!(entries[0].contains("Hello"));
    assert!(!entries[0].contains(r#""action":"read_range""#));
}

#[test]
fn observed_contract_json_includes_semantic_kind_and_locator_hint() {
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读一下 README.md 开头，然后用一句话总结".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    assert!(contract.contains(r#""semantic_kind":"content_excerpt_summary""#));
    assert!(contract.contains(r#""locator_hint":"README.md""#));
}

#[test]
fn observed_request_language_hint_follows_current_user_text() {
    assert_eq!(
        observed_request_language_hint("读一下 README 开头，三句话总结"),
        "zh-CN"
    );
    assert_eq!(
        observed_request_language_hint("Summarize the README in one sentence."),
        "en"
    );
    assert_eq!(observed_request_language_hint("只输出路径"), "zh-CN");
    assert_eq!(observed_request_language_hint("12345"), "config_default");
}

#[test]
fn observed_bilingual_templates_defer_non_bilingual_missing_field_answers() {
    assert!(!observed_language_supports_bilingual_template("ja"));
    assert!(observed_request_prefers_english_template(None, "ja"));
    let missing = serde_json::json!({
        "action": "extract_field",
        "field_path": "package.no_such_key_100_matrix",
        "exists": false,
    });

    assert_eq!(
        extract_field_direct_answer_candidate(
            None,
            &missing,
            Some(OutputResponseShape::OneSentence),
            false,
            true,
        )
        .as_deref(),
        Some("未找到 package.no_such_key_100_matrix 字段")
    );
    assert!(extract_field_direct_answer_candidate(
        None,
        &missing,
        Some(OutputResponseShape::OneSentence),
        true,
        false,
    )
    .is_none());
}

#[test]
fn observed_direct_answer_defers_non_bilingual_existence_with_path_template() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"kind":"missing","path":"/tmp/rustclaw-missing-ja.txt"}],"include_missing":true}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.resolved_intent =
            "ファイルパス /tmp/rustclaw-missing-ja.txt の存在確認。存在しない場合は日本語で短く回答する。"
                .to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "/tmp/rustclaw-missing-ja.txt".to_string();
    let agent_run_context = AgentRunContext {
            original_user_request: Some(
                "/tmp/rustclaw-missing-ja.txt が存在するか確認してください。存在しない場合は日本語で短く答えてください。"
                    .to_string(),
            ),
            route_result: Some(route_result),
            ..AgentRunContext::default()
        };

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
    assert!(extract_direct_answer_from_generic_output_i18n(
        &loop_state,
        &AppState::test_default_with_fixture_provider(),
        Some(&agent_run_context)
    )
    .is_none());
    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
    assert!(extract_direct_scalar_from_generic_output_i18n(
        &loop_state,
        &AppState::test_default_with_fixture_provider(),
        Some(&agent_run_context)
    )
    .is_none());
}

#[test]
fn observed_response_style_hint_reflects_output_contract_shape() {
    let mut route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读一下 README.md 开头，然后用一句话总结".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let mut agent_run_context = AgentRunContext {
        route_result: Some(route_result.clone()),
        ..AgentRunContext::default()
    };
    assert!(observed_response_style_hint(Some(&agent_run_context)).contains("exactly one sentence"));
    assert!(
        observed_response_style_hint(Some(&agent_run_context)).contains(
            "If the request has multiple deliverables, include all of them in that one sentence"
        )
    );

    route_result.output_contract.exact_sentence_count = Some(3);
    agent_run_context.route_result = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context)).contains("exactly 3 sentences"));
    route_result.output_contract.exact_sentence_count = None;

    route_result.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route_result.output_contract.response_shape = OutputResponseShape::Strict;
    route_result.output_contract.exact_sentence_count = Some(1);
    agent_run_context.route_result = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context)).contains("key=value"));
    route_result.output_contract.exact_sentence_count = None;
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;

    route_result.output_contract.response_shape = OutputResponseShape::Scalar;
    agent_run_context.route_result = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("only the final scalar value"));

    route_result.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    agent_run_context.route_result = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("overrides response_shape=scalar"));

    route_result.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route_result.output_contract.response_shape = OutputResponseShape::OneSentence;
    agent_run_context.route_result = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("Do not collapse component counts"));

    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route_result.output_contract.response_shape = OutputResponseShape::Free;
    agent_run_context.route_result = Some(route_result.clone());
    assert!(observed_response_style_hint(Some(&agent_run_context)).contains("raw observed output"));
    assert!(route_disallows_direct_observation_passthrough(
        agent_run_context.route_result.as_ref().unwrap()
    ));
    assert!(observed_contract_json(Some(&agent_run_context))
        .contains(r#""direct_observation_passthrough_allowed":false"#));

    route_result.output_contract.response_shape = OutputResponseShape::FileToken;
    agent_run_context.route_result = Some(route_result);
    assert!(observed_response_style_hint(Some(&agent_run_context)).contains("delivery token"));
}

#[test]
fn chat_wrapped_free_unclassified_contract_allows_finalizer_passthrough() {
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    assert!(!route_requires_synthesized_delivery(&route));

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    assert!(contract.contains(r#""direct_observation_passthrough_allowed":true"#));
    assert!(observed_response_style_hint(Some(&agent_run_context)).contains("short direct answer"));
}

#[test]
fn single_file_delivery_uses_path_batch_fact_as_file_token() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-observed-file-delivery-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("release_checklist.md");
    std::fs::write(&file, "release checklist").expect("write temp file");

    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::FileToken);
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file.display().to_string();

    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        &serde_json::json!({
            "action": "path_batch_facts",
            "count": 1,
            "facts": [
                {
                    "exists": true,
                    "fact": {
                        "kind": "file",
                        "path": file.display().to_string(),
                        "resolved_path": file.display().to_string(),
                        "size_bytes": 17
                    },
                    "path": file.display().to_string()
                }
            ],
            "include_missing": true
        })
        .to_string(),
    ));
    loop_state.has_tool_or_skill_output = true;

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("file token candidate");

    assert_eq!(answer, format!("FILE:{}", file.display()));

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn single_file_delivery_ignores_prior_read_range_rejections_after_path_fact() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-observed-file-delivery-after-reject-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("release_checklist.md");
    std::fs::write(&file, "release checklist").expect("write temp file");

    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::FileToken);
    route.wants_file_delivery = false;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = file.display().to_string();

    let contract_error = "__RC_SKILL_ERROR__:{\"error_kind\":\"contract_action_rejected\",\"error_text\":\"action `system_basic.read_range` is rejected by contract `generic_delivery` (rejected_not_allowed)\",\"extra\":{\"action\":\"system_basic.read_range\",\"contract_match\":\"generic_delivery\",\"decision\":\"rejected_not_allowed\"},\"skill\":\"system_basic\"}";
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(error_step("step_1", "system_basic", contract_error));
    loop_state
        .executed_step_results
        .push(error_step("step_2", "system_basic", contract_error));
    loop_state.executed_step_results.push(ok_step(
        "step_3",
        "fs_basic",
        &serde_json::json!({
            "action": "path_batch_facts",
            "count": 1,
            "facts": [
                {
                    "exists": true,
                    "fact": {
                        "kind": "file",
                        "path": file.display().to_string(),
                        "resolved_path": file.display().to_string(),
                        "size_bytes": 17
                    },
                    "path": file.display().to_string()
                }
            ],
            "include_missing": true
        })
        .to_string(),
    ));
    loop_state.has_tool_or_skill_output = true;
    loop_state.has_recoverable_failure_context = true;

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("file token candidate after rejected reads");

    assert_eq!(answer, format!("FILE:{}", file.display()));

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn chat_wrapped_one_sentence_unclassified_contract_requires_synthesized_delivery() {
    let route = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    assert!(route_requires_synthesized_delivery(&route));

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    assert!(contract.contains(r#""direct_observation_passthrough_allowed":false"#));
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("Do not answer by copying only the raw observed output"));
}

#[test]
fn chat_wrapped_strict_exact_sentence_contract_requires_synthesized_delivery() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.exact_sentence_count = Some(1);
    assert!(route_requires_synthesized_delivery(&route));

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    assert!(contract.contains(r#""direct_observation_passthrough_allowed":false"#));
    assert!(observed_response_style_hint(Some(&agent_run_context))
        .contains("Do not answer by copying only the raw observed output"));
}

#[test]
fn strict_plain_observation_contract_allows_passthrough() {
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    assert!(!route_requires_synthesized_delivery(&route));

    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "model_io.log.2026-05-14 215M\nmodel_io.log.2026-05-11 149M\n",
    ));
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("strict plain observation passthrough");

    assert_eq!(
        answer,
        "model_io.log.2026-05-14 215M\nmodel_io.log.2026-05-11 149M"
    );
}

#[test]
fn raw_command_contract_allows_observation_passthrough() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    assert!(!route_requires_synthesized_delivery(&route));

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let contract = observed_contract_json(Some(&agent_run_context));
    assert!(contract.contains(r#""direct_observation_passthrough_allowed":true"#));
}

#[test]
fn direct_observation_passthrough_detector_matches_raw_output() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "/home/guagua/rustclaw\n"));

    assert!(answer_is_direct_observation_passthrough(
        "/home/guagua/rustclaw",
        &loop_state
    ));
    assert!(!answer_is_direct_observation_passthrough(
        "Working directory: /home/guagua/rustclaw",
        &loop_state
    ));
}

#[test]
fn route_observation_facts_pin_resolved_path_for_existence_summary() {
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "check service file and explain purpose".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPathSummary,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let ctx = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/home/guagua/rustclaw/rustclaw.service".to_string()),
        ..AgentRunContext::default()
    };

    let facts = route_observation_facts_entry(Some(&ctx)).expect("route facts");

    assert!(facts.contains("resolved_target_path: /home/guagua/rustclaw/rustclaw.service"));
    assert!(facts.contains("do not infer the target path from file content fields"));
}

#[test]
fn observed_fallback_prompt_renders_language_and_response_style_hints() {
    let prompt = crate::render_prompt_template(
            OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE,
            &[
                ("__USER_REQUEST__", "读一下 README 开头，然后用一句话总结"),
                (
                    "__RESOLVED_USER_INTENT__",
                    "读一下 README 开头，然后用一句话总结",
                ),
                (
                    "__OUTPUT_CONTRACT__",
                    r#"{"response_shape":"one_sentence","semantic_kind":"content_excerpt_summary"}"#,
                ),
                (
                    "__OBSERVED_OUTPUTS__",
                    "### step_1 skill(read_file)\n# RustClaw",
                ),
                ("__CONFIG_RESPONSE_LANGUAGE__", "zh-CN"),
                ("__REQUEST_LANGUAGE_HINT__", "mixed"),
                (
                    "__RESPONSE_STYLE_HINT__",
                    "Return exactly one sentence unless the current user request explicitly asks for another exact sentence count. If the request has multiple deliverables, include all of them in that one sentence.",
                ),
            ],
        );
    assert!(prompt.contains("Request language hint:\nmixed"));
    assert!(prompt.contains("Response style hint:"));
    assert!(prompt.contains("Return exactly one sentence"));
    assert!(prompt.contains("include all of them in that one sentence"));
    assert!(prompt.contains("Do not collapse multi-dimensional structured evidence"));
    assert!(prompt.contains("combine the deliverables into one grammatical sentence"));
}

#[test]
fn markdown_non_json_fallback_prefers_text_outside_code_fences() {
    let answer = super::non_code_markdown_text(
        "```bash\n#!/usr/bin/env bash\nset -euo pipefail\n```\n\n这个脚本用于重启 clawd 服务。",
    );
    assert_eq!(answer.as_deref(), Some("这个脚本用于重启 clawd 服务。"));
}

#[test]
fn content_excerpt_summary_is_not_hard_summarized_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","excerpt":"12|# timeout note\n13|task_timeout_seconds = 3600\n14|# end"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读取 /tmp/config.toml 最后 3 行，然后用一句话总结".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "/tmp/config.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn content_excerpt_with_summary_composes_observed_slice_and_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"range","start_line":6,"end_line":8,"excerpt":"6|{\"status\":\"ok\",\"prompt_source\":\"clarify\"}\n7|{\"status\":\"ok\",\"prompt_source\":\"dynamic_guard\"}\n8|{\"status\":\"ok\",\"prompt_source\":\"context\"}"}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route_result.output_contract.response_shape = OutputResponseShape::Strict;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = super::compose_content_excerpt_with_summary_answer(
        "All observed records are ok.",
        &loop_state,
        true,
        Some(&agent_run_context),
    );

    assert!(answer.contains(r#""prompt_source":"clarify""#));
    assert!(answer.contains(r#""prompt_source":"dynamic_guard""#));
    assert!(answer.contains(r#""prompt_source":"context""#));
    assert!(answer.contains("All observed records are ok."));
}

#[test]
fn content_excerpt_with_summary_does_not_prepend_log_excerpt() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"tail","requested_n":5,"path":"logs/clawd.run.log","resolved_path":"/workspace/logs/clawd.run.log","excerpt":"1700|2026-05-27T08:04:44Z INFO task_call\n1701|2026-05-27T08:04:45Z INFO task_journal_summary {\"kind\":\"ask\"}\n1702|2026-05-27T08:04:46Z WARN memory_intent"}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route_result.output_contract.response_shape = OutputResponseShape::Strict;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/workspace/logs/clawd.run.log".to_string()),
        ..AgentRunContext::default()
    };

    let answer = super::compose_content_excerpt_with_summary_answer(
        "没有 ERROR 行",
        &loop_state,
        false,
        Some(&agent_run_context),
    );

    assert_eq!(answer, "没有 ERROR 行");
}

#[test]
fn content_excerpt_with_summary_strips_log_excerpt_prefix() {
    let mut loop_state = LoopState::new(2);
    let excerpt = "2026-05-27T08:04:44Z INFO task_call\n2026-05-27T08:04:45Z WARN memory_intent";
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        &format!(
            r#"{{"action":"read_range","mode":"tail","requested_n":2,"path":"logs/clawd.run.log","resolved_path":"/workspace/logs/clawd.run.log","excerpt":"1|{}"}}"#,
            excerpt.replace('\n', r"\n2|")
        ),
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route_result.output_contract.response_shape = OutputResponseShape::Strict;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/workspace/logs/clawd.run.log".to_string()),
        ..AgentRunContext::default()
    };

    let answer = super::compose_content_excerpt_with_summary_answer(
        &format!("{excerpt}\n\n最后 2 行中没有 ERROR 行。"),
        &loop_state,
        false,
        Some(&agent_run_context),
    );

    assert_eq!(answer, "最后 2 行中没有 ERROR 行。");
}

#[test]
fn content_excerpt_with_summary_prefers_auto_locator_slice_over_latest_read() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","mode":"head","requested_n":3,"resolved_path":"/tmp/service_notes.md","excerpt":"1|# Service Notes\n2|Runtime status lives here.\n3|Use this for service checks."}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","mode":"head","requested_n":3,"resolved_path":"/tmp/README.md","excerpt":"1|# Device Local Fixture\n2|This repository contains the sample project.\n3|It is used for filesystem tests."}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptWithSummary;
    route_result.output_contract.requires_content_evidence = true;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/service_notes.md".to_string()),
        ..AgentRunContext::default()
    };

    let answer = super::compose_content_excerpt_with_summary_answer(
        "README.md describes the sample project.",
        &loop_state,
        true,
        Some(&agent_run_context),
    );

    assert!(answer.starts_with("# Service Notes"), "answer: {answer}");
    assert!(answer.contains("README.md describes the sample project."));
    assert!(
        !answer.starts_with("# Device Local Fixture"),
        "answer: {answer}"
    );
}

#[test]
fn direct_answer_keeps_fallback_for_unstructured_content_excerpt_summary() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "read_file",
        "RustClaw is deployed locally and keeps task state in sqlite.",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "看一下 /tmp/README.txt，然后用一句话总结".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "/tmp/README.txt".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.txt".to_string()),
        ..AgentRunContext::default()
    };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("service_status health_check should expose diagnostic machine fields directly");
    assert!(answer.contains("health_check.summary"));
    assert!(answer.contains("clawd.status=running"));
    assert!(answer.contains("clawd_process_count=1"));
    assert!(answer.contains("clawd_health_port_open=true"));
    assert!(answer.contains("clawd_log.keyword_error_count=43"));
    assert!(answer.contains("system_health.load_avg_1m=3.81"));
    assert!(answer.contains("system_health.memory_available_bytes=11270471680"));
    assert!(answer.contains("system_health.disk_root_available_bytes=18108059648"));
}

#[test]
fn direct_answer_summarizes_doc_parse_content_excerpt_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "doc_parse",
            r##"{"text":"# RustClaw\n\n<img src=\"./RustClaw.png\" width=\"420\" />\n\nRustClaw is a local Rust agent runtime centered on clawd and designed for multi-channel task execution.\n\n## Overview\nMore text."}"##,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Read README.md and summarize it in one line.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                "RustClaw is a local Rust agent runtime centered on clawd and designed for multi-channel task execution."
            )
        );
}

#[test]
fn direct_doc_parse_summary_defers_when_language_conflicts_with_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "doc_parse",
            r##"{"text":"# RustClaw\n\nRustClaw is a local Rust agent runtime centered on clawd and designed for multi-channel task execution."}"##,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读取 README.md 并用一句中文总结".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn transform_output_candidate_falls_back_to_result_json() {
    assert_eq!(
        super::transform_skill_formatted_output_candidate(
            r#"{"status":"ok","formatted":null,"result":[{"name":"beta"},{"name":"alpha"}]}"#
        )
        .as_deref(),
        Some(r#"[{"name":"beta"},{"name":"alpha"}]"#)
    );
}

#[test]
fn direct_answer_passthroughs_contract_filename_read_range_excerpt_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|<img src=\"./RustClaw.png\" width=\"420\" />\n4|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "先读一下 README.md 前 4 行".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("# RustClaw\n（空行）\n<img src=\"./RustClaw.png\" width=\"420\" />\n（空行）")
    );
}

#[test]
fn direct_answer_preserves_blank_lines_for_explicit_read_range() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","mode":"range","start_line":1,"end_line":4,"path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|<img src=\"./RustClaw.png\" width=\"420\" />\n4|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Show exactly the first 4 raw lines of README.md.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("# RustClaw\n\n<img src=\"./RustClaw.png\" width=\"420\" />\n")
    );
}

#[test]
fn raw_command_output_read_range_direct_answer_preserves_visible_blank_line() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","mode":"head","requested_n":2,"path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "读取 README.md 前 2 行".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "semantic_contract_requires_evidence".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("# RustClaw\n（空行）")
    );
}

#[test]
fn direct_answer_sanitizes_read_range_log_excerpt_without_llm() {
    let mut loop_state = LoopState::new(2);
    let skill_output = serde_json::json!({
            "action": "read_range",
            "path": "/tmp/feishud.log",
            "resolved_path": "/tmp/feishud.log",
            "excerpt": "1|\u{1b}[32mconnected\u{1b}[0m to wss://host/ws?device_id=123&access_key=abc123&service_id=7&ticket=deadbeef"
        })
        .to_string();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "system_basic", &skill_output));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "看日志最后 1 行".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "feishud.log".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/feishud.log".to_string()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("read_range direct answer");

    assert!(answer.contains("access_key=[REDACTED]"));
    assert!(answer.contains("ticket=[REDACTED]"));
    assert!(!answer.contains('\u{1b}'));
    assert!(!answer.contains("abc123"));
    assert!(!answer.contains("deadbeef"));
}

#[test]
fn scalar_route_fs_basic_tail_read_range_prefers_structured_excerpt() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "older output mentioning scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
    ));
    let skill_output = serde_json::json!({
            "action": "read_range",
            "path": "/home/guagua/rustclaw/logs/clawd.log",
            "resolved_path": "/home/guagua/rustclaw/logs/clawd.log",
            "mode": "tail",
            "requested_n": 2,
            "excerpt": "1858|2026-05-13T18:29:58Z finalize_ok\n1859|2026-05-13T18:29:59Z prior task mentioned release_checklist.md"
        })
        .to_string();
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "fs_basic", &skill_output));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "查看 logs 目录下第二个文件（clawd.log）的最后2行内容".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    assert!(scalar_route_prefers_structured_observed_answer(
        &route_result,
        &loop_state
    ));
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("fs_basic read_range direct answer");

    assert!(answer.contains("finalize_ok"));
    assert!(answer.contains("release_checklist.md"));
    assert!(!answer.contains(r#""action":"read_range""#));
    assert!(!answer.contains("older output mentioning"));
}

#[test]
fn direct_answer_passthroughs_chat_wrapped_execution_path_read_range_when_no_transform_is_requested(
) {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","excerpt":"1|[app]\n2|name = \"fixture\"\n3|mode = \"test\""}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "用户提供了文件路径 /tmp/config.toml，但未说明要对该文件执行什么操作"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/tmp/config.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/config.toml".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("[app]\nname = \"fixture\"\nmode = \"test\"")
    );
}

#[test]
fn direct_answer_does_not_passthrough_read_range_when_summary_is_requested() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|A tool runtime\n4|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "先读一下 README.md 前 4 行，再用三句话总结".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_filename_read_range".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };
    assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none(),
            "summary-style read_range requests should fall back to synthesis instead of raw passthrough"
        );
}

#[test]
fn direct_answer_defers_read_range_passthrough_when_language_conflicts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"read_range","path":"/tmp/service_notes.md","resolved_path":"/tmp/service_notes.md","excerpt":"1|# Service Notes\n2|\n3|RustClaw test fixture service notes."}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "service_notes.md 를 읽고 핵심만 요약해.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_filename_read_range".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "service_notes.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/service_notes.md".to_string()),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none(),
        "language-conflicting read_range evidence should be synthesized instead of raw passthrough"
    );
}

#[test]
fn direct_answer_does_not_passthrough_read_range_for_existence_with_path_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/rustclaw.service","resolved_path":"/tmp/rustclaw.service","excerpt":"1|[Unit]\n2|Description=RustClaw Service\n3|[Service]\n4|ExecStart=/bin/bash start-all-bin.sh"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查 rustclaw.service 是否存在，若存在返回路径并解释用途".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/rustclaw.service".to_string()),
        ..AgentRunContext::default()
    };

    assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none(),
            "existence/path contracts with read_range evidence need synthesis, not raw file passthrough"
        );
}

#[test]
fn direct_answer_prefers_current_turn_excerpt_summary_request_over_resolved_intent_drift() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"/tmp/README.md","resolved_path":"/tmp/README.md","excerpt":"1|# RustClaw\n2|\n3|A tool runtime\n4|"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "先读一下 README.md 前 4 行".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_filename_read_range".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Filename,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            locator_hint: "README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        user_request: Some("先读一下 README.md 前 4 行，再用三句话总结".to_string()),
        route_result: Some(route_result),
        auto_locator_path: Some("/tmp/README.md".to_string()),
        ..AgentRunContext::default()
    };
    assert!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .is_none(),
            "current-turn summary/read-range request should still block raw passthrough even if resolved_intent drifted"
        );
}

#[test]
fn direct_answer_formats_structured_keys_result_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读 /tmp/package.json，告诉我 scripts 字段下都有哪些子键".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_explicit_path_structured_keys".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/tmp/package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("build\ndev\nlint")
    );
}

#[test]
fn direct_answer_formats_structured_keys_presence_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"structured_keys","path":"/tmp/en-US.toml","resolved_path":"/tmp/en-US.toml","field_path":"","exists":true,"container_type":"object","count":3,"keys":["execute_prefixes","locale","result_suffixes"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读取 /tmp/en-US.toml 并确认是否存在 negative_markers 字段".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "planner_locator_requires_evidence".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::StructuredKeys,
            locator_hint: "/tmp/en-US.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        original_user_request: Some(
            "读取 configs/command_intent/en-US.toml，只回答是否还有 negative_markers 字段"
                .to_string(),
        ),
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("不包含 negative_markers 字段")
    );
}

#[test]
fn direct_answer_formats_structured_array_identity_presence_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"structured_keys","path":"/tmp/skills_registry.toml","resolved_path":"/tmp/skills_registry.toml","field_path":"skills","exists":true,"container_type":"array","count":2,"identity_values":["fs_basic","config_basic"],"identity_omitted":0,"indices_preview":[{"index":0,"value_type":"object","keys":["name","planner_kind"],"identity_key":"name","identity_value":"fs_basic"},{"index":1,"value_type":"object","keys":["name","planner_kind"],"identity_key":"name","identity_value":"config_basic"}]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读取 /tmp/skills_registry.toml，回答 fs_basic 是否注册".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "planner_locator_requires_evidence".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::StructuredKeys,
            locator_hint: "/tmp/skills_registry.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        original_user_request: Some(
            "读取 docker/config/skills_registry.toml，回答 fs_basic 是否注册".to_string(),
        ),
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("包含 fs_basic")
    );
}

#[test]
fn structured_keys_one_sentence_defers_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读 /tmp/package.json，用一句话告诉我 scripts 字段下有哪些子键"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_explicit_path_structured_keys".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/tmp/package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_formats_extract_fields_result_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_fields","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","count":2,"results":[{"field_path":"database.sqlite_path","exists":true,"value_type":"string","value_text":"data/rustclaw.db","value":"data/rustclaw.db"},{"field_path":"tools.allow_sudo","exists":true,"value_type":"bool","value_text":"true","value":true}]}"#,
        ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "读取 /tmp/config.toml 里的 database.sqlite_path 和 tools.allow_sudo，告诉我两个字段的值"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:generic_explicit_path_extract_fields"
                .to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: OutputSemanticKind::None,
                locator_hint: "/tmp/config.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("database.sqlite_path: data/rustclaw.db\ntools.allow_sudo: true")
    );
}

#[test]
fn direct_answer_uses_inventory_dir_names_for_system_basic() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["act_plan.log","clawd.log","feishud.log"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("act_plan.log\nclawd.log\nfeishud.log")
    );
}

#[test]
fn direct_answer_uses_inventory_dir_names_for_fs_basic() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","path":"/tmp/document","resolved_path":"/tmp/document","files_only":true,"names_only":true,"names":["a.txt","b.md","c.png"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "List file names from a known directory.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "document".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a.txt\nb.md\nc.png")
    );
}

#[test]
fn direct_answer_uses_inventory_dir_entry_sizes_when_names_only_is_false() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":false,"entries":[{"name":"act_plan.log","kind":"file","size_bytes":2467002},{"name":"clawd.run.log","kind":"file","size_bytes":397321},{"name":"clawd.log","kind":"file","size_bytes":2035}],"names":["act_plan.log","clawd.run.log","clawd.log"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下最大的 3 个文件，输出文件名和大小".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("act_plan.log 2467002\nclawd.run.log 397321\nclawd.log 2035")
    );
}

#[test]
fn direct_answer_does_not_apply_listing_limit_from_resolved_intent_text() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["a","b","c","d"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a\nb\nc\nd")
    );
}

#[test]
fn direct_answer_does_not_apply_listing_limit_from_current_turn_request_text() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/logs","resolved_path":"/tmp/logs","names_only":true,"names":["a","b","c","d"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下的文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer:planner_execute_chat_wrapped".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some("列出 logs 目录下前 2 个文件名".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a\nb\nc\nd")
    );
}

#[test]
fn scalar_listing_gate_does_not_repair_count_from_request_text_limit() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a\nb\nc\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下的文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarCount,
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some("列出 logs 目录下前 2 个文件名，只输出文件名".to_string()),
        ..AgentRunContext::default()
    };
    let route = agent_run_context.route_result.as_ref().unwrap();
    assert!(
        !super::scalar_route_prefers_structured_observed_answer(route, &loop_state,),
        "scalar/listing gate must not infer bounded listing from current-turn request text"
    );
}

#[test]
fn direct_answer_uses_latest_list_dir_entries_for_act_free_shape() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "README.txt\nnotes.md\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 archive 目录下有什么".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "archive".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("README.txt\nnotes.md")
    );
}

#[test]
fn direct_answer_uses_latest_list_dir_even_after_synthesis_step() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "alpha.md\nbeta.md\n"));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "synthesize_answer",
        "document 目录下有 alpha.md 和 beta.md。",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 document 目录下有哪些文件，只输出文件名列表".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::FileNames,
            locator_hint: "document".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some("列出 document 目录下有哪些文件，只输出文件名列表".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("alpha.md\nbeta.md")
    );
}

#[test]
fn direct_answer_preserves_list_dir_entries_without_request_text_limit() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a\nb\nc\nd\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a\nb\nc\nd")
    );
}

#[test]
fn direct_answer_defers_hidden_entries_explanation_shape_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".git\nREADME.md\n.env\nsrc\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "检查当前目录是否存在隐藏文件，然后用一句话解释隐藏文件的常见用途"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_formats_hidden_entries_check_scalar_from_listing() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".git\nREADME.md\n.env\nsrc\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("2")
    );
}

#[test]
fn direct_answer_formats_hidden_entries_check_strict_shape_from_listing() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".\n..\n.codex\n.git/\n.gitignore\nREADME.md\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(".codex\n.git/\n.gitignore")
    );
}

#[test]
fn direct_answer_formats_hidden_entries_check_empty_inventory_without_followup() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","counts":{"dirs":1,"files":1,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","name":"README.md","path":"README.md"},{"hidden":false,"kind":"dir","name":"src","path":"src"}],"include_hidden":true,"names":["README.md","src"],"path":"/tmp/workspace"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.route_reason =
        "structured_contract_hint_fast_path; contract_hint_fast_path".to_string();
    route_result.resolved_intent = "检查当前目录有没有隐藏文件，如果有就列出几个例子。".to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("hidden entries strict contract should answer from inventory");

    assert!(answer.contains("未发现隐藏文件"));
    assert!(!answer.contains("要继续"));
}

#[test]
fn direct_answer_defers_hidden_entries_check_free_shape_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        ".cargo/\nREADME.md\n.dockerignore\n.env.example\nsrc\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_defers_hidden_entries_check_one_sentence_from_system_basic_inventory_dir() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/workspace","resolved_path":"/tmp/workspace","names_only":true,"include_hidden":true,"names":[".cargo",".dockerignore",".env.example","README.md","src"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_formats_existence_with_path_from_system_basic_path_batch_facts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/tmp/rustclaw-workspace/rustclaw.service","size_bytes":1190},"path":"/tmp/rustclaw-workspace/rustclaw.service"}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("有，路径：/tmp/rustclaw-workspace/rustclaw.service")
    );
}

#[test]
fn direct_answer_formats_strict_path_kind_from_fs_basic_path_batch_facts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"dir","path":"configs/channels","resolved_path":"/tmp/repo/configs/channels","size_bytes":4096},"path":"/tmp/repo/configs/channels"}],"include_missing":true}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.resolved_intent = "查看 configs 目录下最后一个条目的路径和类型信息".to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "/tmp/repo/configs/channels".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/repo/configs/channels | 目录")
    );
    assert!(observed_output_entries(&loop_state)
        .join("\n")
        .contains("kind=dir"));
}

#[test]
fn direct_answer_formats_multi_path_facts_without_llm_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","path":"package.json","resolved_path":"/tmp/repo/package.json","size_bytes":120},"path":"package.json"},{"exists":false,"path":"nope.json","error":"not found"}],"include_missing":true}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.resolved_intent =
        "Inspect explicit file paths and answer with existence and type".to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route_result.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route_result.output_contract.locator_hint = "/tmp/repo".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("multi path facts answer");
    assert!(answer.contains("/tmp/repo/package.json: exists, type file"));
    assert!(answer.contains("nope.json: not found"));
}

#[test]
fn direct_answer_formats_scalar_existence_without_path_from_system_basic_path_batch_facts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"configs/config.toml","resolved_path":"/tmp/repo/configs/config.toml","size_bytes":1190},"path":"/tmp/repo/configs/config.toml"}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查 configs/config.toml 是否存在，只回答有或没有".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "configs/config.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("有")
    );
}

#[test]
fn direct_answer_formats_path_batch_facts_requested_size() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"fields":["exists","size"],"facts":[{"exists":true,"fact":{"kind":"file","path":"data/rustclaw.db","resolved_path":"/tmp/repo/data/rustclaw.db","size_bytes":55226368},"path":"/tmp/repo/data/rustclaw.db"}],"include_missing":true}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.ask_mode = crate::AskMode::planner_execute_plain();
    route_result.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "data/rustclaw.db".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("yes, path: /tmp/repo/data/rustclaw.db, size: 55226368 bytes")
    );
}

#[test]
fn direct_answer_formats_missing_path_batch_facts_with_reason() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":false,"path":"/tmp/missing.txt","error":"not found"}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查文件 /tmp/missing.txt 是否存在，如果不存在，简短说明原因。"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "/tmp/missing.txt".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("missing path answer");

    assert!(answer.contains("路径不存在"));
    assert!(answer.contains("/tmp/missing.txt"));
}

#[test]
fn direct_answer_formats_existence_with_path_from_run_cmd_yes_output() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_exists_yes_{}_{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let target = temp_dir.join("rustclaw.service");
    std::fs::write(&target, "ok").expect("write target");
    let expected = format!(
        "有，路径：{}",
        target
            .canonicalize()
            .unwrap_or(target.clone())
            .to_string_lossy()
    );

    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "yes\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(expected.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_formats_existence_with_path_from_run_cmd_exists_output() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_exists_lower_{}_{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let target = temp_dir.join("rustclaw.service");
    std::fs::write(&target, "ok").expect("write target");
    let expected = format!(
        "有，路径：{}",
        target
            .canonicalize()
            .unwrap_or(target.clone())
            .to_string_lossy()
    );

    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "exists\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(expected.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_formats_existence_with_path_from_system_basic_find_name_output() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_exists_find_name_{}_{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let target = temp_dir.join("rustclaw.service");
    std::fs::write(&target, "ok").expect("write target");
    let resolved = target
        .canonicalize()
        .unwrap_or(target.clone())
        .to_string_lossy()
        .to_string();

    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    let expected = format!("有，路径：{resolved}");
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(expected.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_does_not_passthrough_listing_when_content_evidence_is_required() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "list_dir",
        "base_skill_response_contract.md\nskill_integration_guide.md\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_does_not_passthrough_inventory_dir_when_content_evidence_is_required() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/docs","resolved_path":"/tmp/docs","names_only":true,"names":["base_skill_response_contract.md","skill_integration_guide.md"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_does_not_passthrough_run_cmd_listing_when_content_evidence_is_required() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-observed-output-listing-only-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "a.md\nb.md\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_blocks_contract_forbidden_observation_action() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "hello from shell"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "读取 docs/guide.md 并总结".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "docs/guide.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn directory_purpose_summary_is_not_hard_classified_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/docs","resolved_path":"/tmp/docs","names_only":true,"names":["release_checklist.md","operator-guide.md","rollout-summary.pdf"]}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列出 docs 目录下的文件，再用一句话解释这些文档大概是干什么的".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn recent_artifacts_judgment_is_not_hard_classified_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "total 151792\n-rw-r--r--@ 1 testuser staff 76509771 Apr 12 16:30 model_io.log\n-rw-r--r--@ 1 testuser staff 906739 Apr 12 16:30 act_plan.log\n-rw-r--r--@ 1 testuser staff 191187 Apr 12 15:48 service_ops.log\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列出 logs 目录最近修改的 3 个文件，再告诉我这更像是测试日志还是正式产物"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_system_basic_info_summary_to_llm_for_brief_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"rustclaw-test-host.local","os":"macos","arch":"x86_64","cwd":"/tmp/rustclaw-workspace"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "show me the basic machine info here like hostname and system, keep it brief"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_archive_creation_success_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "archive_basic",
        "exit=0\nupdating: tmp/rustclaw-workspace/scripts/skill_calls/\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "scripts/skill_calls -> tmp/nl_archive_case.zip".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
    assert!(
        has_observed_answer_candidates(&loop_state),
        "archive output should remain available as observed facts for synthesis"
    );
}

#[test]
fn direct_answer_defers_archive_basic_output_destination_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            r#"{"action":"pack","format":"zip","source":"/tmp/rustclaw-workspace/scripts/skill_calls","archive":"/tmp/rustclaw-workspace/tmp/nl_archive_case.zip","output":"exit=0\nupdating: skill_calls/\n"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，然后告诉我是否成功"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "scripts/skill_calls".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
    assert!(
        has_observed_answer_candidates(&loop_state),
        "archive json should remain available as observed facts for synthesis"
    );
}

#[test]
fn archive_pack_scalar_contract_returns_created_archive_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            "archive_path=/tmp/rustclaw-workspace/tmp/nl_archive_case.zip\nexit=0\n  adding: /tmp/rustclaw-workspace/scripts/skill_calls/ (stored 0%)\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "把 scripts/skill_calls 打成一个 zip 到 tmp/nl_archive_case.zip，只返回生成路径"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ArchivePack,
            locator_hint: "scripts/skill_calls | tmp/nl_archive_case.zip".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/rustclaw-workspace/tmp/nl_archive_case.zip")
    );
}

#[test]
fn archive_unpack_contract_returns_one_sentence_destination_summary() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            "dest_path=/tmp/rustclaw-workspace/tmp/contract_matrix_unpacked\nexit=0\nArchive: /tmp/test_bundle.zip\n inflating: /tmp/rustclaw-workspace/tmp/contract_matrix_unpacked/notes.txt\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "把 test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ArchiveUnpack,
            locator_hint: "/tmp/test_bundle.zip | tmp/contract_matrix_unpacked".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("已解压到 /tmp/rustclaw-workspace/tmp/contract_matrix_unpacked，包含 notes.txt。")
    );
}

#[test]
fn direct_answer_defers_system_basic_info_summary_without_action_field() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"hostname":"rustclaw-test-host.local","os":"macos","arch":"x86_64","cwd":"/tmp/rustclaw-workspace"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "show me the basic machine info here like hostname and system, keep it brief"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_system_basic_info_for_free_shape_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"ThinkPad-X1","os":"linux","arch":"x86_64","cwd":"/home/guagua/rustclaw"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "show me the basic machine info here like hostname and system, keep it brief"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_extracts_cwd_from_system_basic_info_for_scalar_path_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"info","hostname":"ThinkPad-X1","os":"linux","arch":"x86_64","cwd":"/home/guagua/rustclaw","workspace_root":"/home/guagua/rustclaw"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "获取当前工作目录的绝对路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:scalar_path_only".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/home/guagua/rustclaw")
    );
}

#[test]
fn direct_scalar_extracts_cwd_from_system_basic_info_without_action_field() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"hostname":"ThinkPad-X1","os":"linux","arch":"x86_64","cwd":"/home/guagua/rustclaw","workspace_root":"/home/guagua/rustclaw"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "获取当前工作目录的绝对路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:scalar_path_only".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/home/guagua/rustclaw")
    );
}

#[test]
fn direct_scalar_path_contract_prefers_recorded_write_file_path() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "/home/guagua/rustclaw"));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "write_file",
        "written 40 bytes to /home/guagua/rustclaw/document/pwd_line.txt",
    ));
    loop_state.output_vars.insert(
        "last_file_path".to_string(),
        "/home/guagua/rustclaw/document/pwd_line.txt".to_string(),
    );
    loop_state.last_written_file_path =
        Some("/home/guagua/rustclaw/document/pwd_line.txt".to_string());
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "create the file and send me the file path only".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:scalar_path_only".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ScalarPathOnly,
            locator_hint: "pwd_line.txt".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/home/guagua/rustclaw/document/pwd_line.txt")
    );
}

#[test]
fn workspace_project_summary_is_not_hard_summarized_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "Cargo.toml\ncrates/\nUI/\nconfigs/\nREADME.md\nREADME.zh-CN.md\nprompts/\nrustclaw.service\ncomponent_start/start-telegramd.sh\ncomponent_start/start-wechatd.sh\ncomponent_start/start-whatsappd.sh\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "用非技术用户能听懂的话，简短解释这个仓库主要是干什么的".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_scalar_uses_latest_list_dir_entries_when_listing_is_latest_step() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "README.txt\n"));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("README.txt")
    );
}

#[test]
fn direct_scalar_path_only_uses_auto_locator_full_path_for_unique_list_dir_match() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-observed-output-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let file_path = temp_dir.join("Report.MD");
    std::fs::write(&file_path, "hello").unwrap();

    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "Report.MD\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "去 case_only 找 report.md，只输出路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(file_path.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    let resolved = file_path
        .canonicalize()
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(resolved.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_scalar_path_only_uses_rooted_full_path_for_unique_find_name_match() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-observed-output-find-name-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let file_path = temp_dir.join("Report.MD");
    std::fs::write(&file_path, "hello").unwrap();

    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            &format!(
                r#"{{"action":"find_name","pattern":"report.md","count":1,"results":["Report.MD"],"root":"{}"}}"#,
                temp_dir.display()
            ),
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "去 case_only 找 report.md，只输出路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    let resolved = file_path
        .canonicalize()
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(resolved.as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn system_basic_find_path_normalization_prefers_existing_relative_path() {
    let rel_dir = Path::new("target").join(format!(
        "clawd-observed-output-find-path-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&rel_dir).unwrap();
    let file_path = rel_dir.join("Report.MD");
    std::fs::write(&file_path, "hello").unwrap();
    let cwd = std::env::current_dir().unwrap();
    let resolved_root = cwd.join(&rel_dir).to_string_lossy().to_string();
    let expected = file_path
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();

    assert_eq!(
        normalize_system_basic_match_path(
            Some(&resolved_root),
            Some(file_path.to_string_lossy().as_ref())
        )
        .as_deref(),
        Some(expected.as_str())
    );
    let _ = std::fs::remove_dir_all(rel_dir);
}

#[test]
fn direct_scalar_path_only_prefers_resolved_path_from_path_batch_facts() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","resolved_path":"/tmp/case_only/Report.MD","size_bytes":33},"path":"/tmp/case_only/report.md","resolved_from_case_insensitive":true}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "去 case_only 目录里找 report.md，只输出路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/case_only/Report.MD")
    );
}

#[test]
fn direct_answer_keeps_plain_path_terminal_format_for_observed_path_fact() {
    let mut loop_state = LoopState::new(2);
    loop_state.last_user_visible_respond = Some("/tmp/case_only/Report.MD".to_string());
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"scripts/nl_tests/fixtures/locator_smart/case_only/Report.MD","resolved_path":"/tmp/case_only/Report.MD","size_bytes":33},"path":"/tmp/case_only/Report.MD"}],"include_missing":true}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "去 case_only 目录里找 report.md，只输出路径".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
            locator_hint: "report.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/case_only/Report.MD")
    );
}

#[test]
fn direct_scalar_does_not_passthrough_multiline_list_dir_listing() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "README.txt\nnotes.md\n"));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None),
        None
    );
}

#[test]
fn direct_scalar_counts_multiline_list_dir_when_route_requests_count() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a\nb\nc\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "数一下 scripts 目录直接有多少个子项".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarCount,
            locator_hint: "scripts".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("3")
    );
}

#[test]
fn direct_scalar_uses_inventory_dir_count_for_scalar_count() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"scripts","resolved_path":"/tmp/scripts","names_only":true,"names":["a","b","c"],"counts":{"total":3}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "数一下 scripts 目录直接子项有多少个，只输出数字".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:current_workspace_scalar_count".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarCount,
            locator_hint: "scripts".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("3")
    );
}

#[test]
fn direct_count_uses_inventory_dir_total_for_non_scalar_shape() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","path":"document","resolved_path":"/tmp/document","names_only":true,"names":["a","b","c","d"],"counts":{"total":4,"files":4,"dirs":0},"recursive":false}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "再数一下 document 目录直接有多少个子项".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "scalar count with free-form response shape".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarCount,
            locator_hint: "document".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("4")
    );
}

#[test]
fn direct_scalar_path_lists_inventory_dir_candidates_without_choosing_first() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":"/tmp/stem_multi","resolved_path":"/tmp/stem_multi","names_only":true,"names":["abcd.cpp","abcd.txt"],"counts":{"total":2}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "find matching paths".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "structured scalar path request".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_hint: "/tmp/stem_multi".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("/tmp/stem_multi/abcd.cpp\n/tmp/stem_multi/abcd.txt")
    );
}

#[test]
fn direct_scalar_uses_inventory_dir_hidden_count_for_hidden_entries_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"inventory_dir","path":".","resolved_path":"/tmp/workspace","include_hidden":true,"names_only":true,"names":[".git",".env","README.md"],"counts":{"total":3,"hidden":2}}"#,
        ));
    let route_result = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "数一下当前目录里以点开头的隐藏文件有几个，只输出数字".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:hidden_entries_check".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::HiddenEntriesCheck,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("2")
    );
}

#[test]
fn direct_answer_formats_package_manager_detect_summary() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "package_manager",
        "package_manager=brew",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "看看当前机器识别到的包管理器，再一句话说最可能日常会用哪个".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:package_manager_detect_summary".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("检测到的包管理器是 brew，依据是 package_manager 返回了 package_manager=brew。")
    );
}

#[test]
fn direct_answer_formats_package_manager_matrix_basis_summary() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "package_manager",
        "package_manager=apt-get",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "检测这台机器可用的包管理器，并说明依据。".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:package_manager_detection".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::PackageManagerDetection,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("检测到的包管理器是 apt-get，依据是 package_manager 返回了 package_manager=apt-get。")
    );
}

#[test]
fn direct_scalar_extracts_package_manager_detect_value() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "package_manager",
        "package_manager=brew",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "只输出当前机器识别到的包管理器名称".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:package_manager_detect_scalar".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("brew")
    );
}

#[test]
fn sqlite_database_kind_judgment_is_not_hard_classified_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[]}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "看看 data/db-basic-contract.sqlite 里有哪些表，再一句话说这更像业务库还是测试库"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer:planner_execute_chat_wrapped".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
            locator_hint: "data/db-basic-contract.sqlite".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn sqlite_database_kind_judgment_uses_contract_selector_and_cites_tables() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
        ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "判断 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 更像业务库还是测试库，并给出依据"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:sqlite_database_kind_judgment".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "判断这个 SQLite 更像业务库还是测试库。\n[CONTRACT_TEST_HINT]\nselector_database_kind=test\n[/CONTRACT_TEST_HINT]"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("expected deterministic sqlite database kind answer");
    assert!(answer.contains("更像测试库"), "{answer}");
    assert!(answer.contains("orders"), "{answer}");
    assert!(answer.contains("service_logs"), "{answer}");
    assert!(answer.contains("users"), "{answer}");
    assert!(!answer.contains("第 1 步"), "{answer}");
}

#[test]
fn sqlite_database_kind_judgment_uses_run_cmd_table_names_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "orders\nservice_logs\nusers\n",
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "判断 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 更像业务库还是测试库，并给出依据"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:sqlite_database_kind_judgment".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "判断这个 SQLite 更像业务库还是测试库。\n[CONTRACT_TEST_HINT]\nselector_database_kind=test\n[/CONTRACT_TEST_HINT]"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("expected deterministic run_cmd sqlite database kind answer");
    assert!(answer.contains("更像测试库"), "{answer}");
    assert!(answer.contains("orders"), "{answer}");
    assert!(answer.contains("service_logs"), "{answer}");
    assert!(answer.contains("users"), "{answer}");
}

#[test]
fn sqlite_schema_version_uses_run_cmd_value_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "schema_version=7\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "读取 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 的 schema 版本"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:sqlite_schema_version".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::SqliteSchemaVersion,
            locator_hint: "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
                .to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("schema_version=7")
    );
}

#[test]
fn sqlite_table_listing_uses_run_cmd_table_names_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "orders\nservice_logs\nusers\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "列出 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里的表"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:sqlite_table_listing".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::SqliteTableListing,
            locator_hint: "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
                .to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("| name |\n| --- |\n| orders |\n| service_logs |\n| users |")
    );
}

#[test]
fn sqlite_database_kind_judgment_prefers_table_inventory_over_later_name_columns() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "db_basic",
            r#"{"columns":["id","name","email"],"rows":[{"email":"alice@example.com","id":1,"name":"Alice"},{"email":"bob@example.com","id":2,"name":"Bob"}]}"#,
        ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "判断 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 更像业务库还是测试库，并给出依据"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:sqlite_database_kind_judgment".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "判断这个 SQLite 更像业务库还是测试库。\n[CONTRACT_TEST_HINT]\nselector_database_kind=test\n[/CONTRACT_TEST_HINT]"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("expected deterministic sqlite database kind answer");
    assert!(answer.contains("orders"), "{answer}");
    assert!(answer.contains("service_logs"), "{answer}");
    assert!(answer.contains("users"), "{answer}");
    assert!(!answer.contains("Alice"), "{answer}");
    assert!(!answer.contains("Bob"), "{answer}");
}

#[test]
fn direct_answer_lists_sqlite_table_names_without_llm_when_names_only_is_requested() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent:
                "看一下 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里有哪些表，只输出表名"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:planner_execute_chat_wrapped".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteTableNamesOnly,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("orders\nusers")
    );
}

#[test]
fn direct_scalar_lists_sqlite_table_names_when_names_only_contract_is_scalar() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "看一下 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里有哪些表，只输出表名"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:act".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteTableNamesOnly,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("orders\nusers")
    );
}

#[test]
fn direct_scalar_does_not_take_first_db_row_from_multi_row_query() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Read a scalar value from the SQLite database".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer:act".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "data/app.sqlite".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_scalar_counts_db_rows_for_scalar_count_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.resolved_intent =
            "统计 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 的表数量，只输出数字"
                .to_string();
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("3")
    );
}

#[test]
fn structured_observed_body_preserves_db_table_inventory_instead_of_first_scalar_only() {
    let body = r#"{"columns":["name"],"rows":[{"name":"users"},{"name":"orders"},{"name":"service_logs"}]}"#;
    assert_eq!(
        structured_observed_body("db_basic", body).as_deref(),
        Some("db_tables=users, orders, service_logs")
    );
}

#[test]
fn archive_list_summary_parses_raw_zip_table_for_synthesis() {
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    let summary = archive_list_summary_from_body(body).expect("zip listing should parse");
    assert_eq!(summary.archive.as_deref(), Some("/tmp/test_bundle.zip"));
    assert_eq!(summary.entries.len(), 2);
    assert_eq!(summary.entries[0].name, "notes.txt");
    assert_eq!(summary.entries[0].size_bytes, Some(22));
    assert_eq!(
            structured_observed_body("archive_basic", body).as_deref(),
            Some(
                "archive_basic action=list archive=/tmp/test_bundle.zip total_entries=2\nentry name=notes.txt size_bytes=22\nentry name=nested/config.ini size_bytes=20"
            )
        );
}

#[test]
fn archive_list_observed_fact_survives_artifact_filter() {
    let mut loop_state = LoopState::new(2);
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));

    assert!(
        has_observed_answer_candidates(&loop_state),
        "normalized archive list facts should remain available for synthesis"
    );
}

#[test]
fn archive_read_direct_answer_returns_member_content() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"action":"read","archive":"/tmp/test_bundle.zip","member":"notes.txt","content":"fixture archive notes\n"}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/test_bundle.zip | notes.txt".to_string();

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some("/tmp/test_bundle.zip | notes.txt".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("fixture archive notes")
    );
}

#[test]
fn archive_read_direct_answer_does_not_require_semantic_label() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"action":"read","archive":"/tmp/test_bundle.zip","member":"notes.txt","content":"fixture archive notes\n"}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/test_bundle.zip".to_string();

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some("/tmp/test_bundle.zip".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("fixture archive notes")
    );
}

#[test]
fn archive_raw_passthrough_replacement_uses_structured_summary() {
    let mut loop_state = LoopState::new(2);
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let state = AppState::test_default_with_fixture_provider();
    let replacement = archive_list_raw_passthrough_replacement(body, &state, &loop_state, "zh-CN")
        .expect("raw archive output should be replaced");
    assert!(replacement.contains("压缩包包含 2 个条目"));
    assert!(replacement.contains("notes.txt"));
    assert!(replacement.contains("nested/config.ini"));
    assert!(!replacement.contains("Archive:"));
}

#[test]
fn archive_list_scalar_count_reads_entry_count_directly() {
    let mut loop_state = LoopState::new(2);
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;

    let agent_run_context = AgentRunContext {
        route_result: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("2")
    );
    assert!(scalar_count_diagnostic_line_for_answer("2", Some(&route), &loop_state).is_none());
}

#[test]
fn archive_entry_existence_reads_archive_list_directly() {
    let mut loop_state = LoopState::new(2);
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.resolved_intent =
        "Check whether notes.txt exists in /tmp/test_bundle.zip without extraction.".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/test_bundle.zip".to_string();

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        original_user_request: Some(
            "Only tell me whether notes.txt exists in /tmp/test_bundle.zip; do not extract it."
                .to_string(),
        ),
        auto_locator_path: Some("/tmp/test_bundle.zip".to_string()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_scalar_from_generic_output_i18n(
        &loop_state,
        &AppState::test_default_with_fixture_provider(),
        Some(&agent_run_context),
    )
    .expect("archive member existence should be answered from archive entries");
    assert!(answer.contains("notes.txt"), "answer: {answer}");
    assert!(answer.contains("exists"), "answer: {answer}");
    assert!(!answer.contains("nested/config.ini"), "answer: {answer}");
}

#[test]
fn structured_observed_body_includes_path_batch_metadata_for_synthesis() {
    let body = r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","modified_ts":1777345844,"path":"Cargo.lock","resolved_path":"/tmp/repo/Cargo.lock","size_bytes":121657},"path":"/tmp/repo/Cargo.lock"},{"exists":true,"fact":{"kind":"file","modified_ts":1777357772,"path":"Cargo.toml","resolved_path":"/tmp/repo/Cargo.toml","size_bytes":2606},"path":"/tmp/repo/Cargo.toml"}],"include_missing":true}"#;
    assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "path_batch_facts\npath_fact name=Cargo.lock path=/tmp/repo/Cargo.lock exists=true kind=file size_bytes=121657 modified_ts=1777345844\npath_fact name=Cargo.toml path=/tmp/repo/Cargo.toml exists=true kind=file size_bytes=2606 modified_ts=1777357772"
            )
        );
}

#[test]
fn structured_observed_body_includes_inventory_dir_entry_metadata_for_synthesis() {
    let body = r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},{"hidden":false,"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187}],"names":["intent_normalizer.schema.json","plan_result.schema.json"],"path":"prompts/schemas","resolved_path":"/tmp/repo/prompts/schemas","sort_by":"size_desc"}"#;
    assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "inventory_dir path=/tmp/repo/prompts/schemas sort_by=size_desc total=2 files=2 dirs=0 hidden=0\nentry name=intent_normalizer.schema.json kind=file size_bytes=9402 modified_ts=1777513843\nentry name=plan_result.schema.json kind=file size_bytes=4187 modified_ts=1777526917"
            )
        );
}

#[test]
fn structured_observed_body_compacts_large_inventory_dir_by_kind() {
    let entries = (0..9)
        .map(|idx| {
            serde_json::json!({
                "hidden": false,
                "kind": "dir",
                "modified_ts": 1777513843,
                "name": format!("dir_{idx}"),
                "path": format!("dir_{idx}"),
                "size_bytes": 0
            })
        })
        .chain((0..9).map(|idx| {
            serde_json::json!({
                "hidden": false,
                "kind": "file",
                "modified_ts": 1777513843,
                "name": format!("file_{idx}.md"),
                "path": format!("file_{idx}.md"),
                "size_bytes": 42
            })
        }))
        .collect::<Vec<_>>();
    let body = serde_json::json!({
        "action": "inventory_dir",
        "counts": {"dirs": 9, "files": 9, "hidden": 0, "total": 18},
        "entries": entries,
        "path": ".",
        "resolved_path": "/tmp/repo",
        "sort_by": "name"
    })
    .to_string();

    let observed = structured_observed_body("system_basic", &body).expect("observed body");
    assert!(observed.contains("dir_entries=dir_0:size_bytes=0,dir_1:size_bytes=0"));
    assert!(observed.contains("file_entries=file_0.md:size_bytes=42,file_1.md:size_bytes=42"));
    assert!(!observed.contains("modified_ts=1777513843"));
    assert!(observed.contains("size_bytes=42"));
}

#[test]
fn structured_observed_body_includes_count_inventory_breakdown_for_synthesis() {
    let body = r#"{"action":"count_inventory","counts":{"dirs":26,"files":40,"hidden":0,"total":66},"kind_filter":"any","path":".","resolved_path":"/tmp/repo"}"#;
    assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "action=count_inventory\npath=.\nresolved_path=/tmp/repo\nkind_filter=any\ncount_files=40\ncount_dirs=26\ncount_total=66\ncount_hidden=0"
            )
        );
}

#[test]
fn sqlite_table_listing_summary_defers_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "列一下 data/app.sqlite 里有哪些表".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer:planner_execute_chat_wrapped".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::SqliteTableListing,
            locator_hint: "data/app.sqlite".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_scalar_defers_route_locator_hint_quantity_comparison_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a\nb\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "list_dir", "a\nb\nc\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "上一个和上上个哪个更多，只回答目录名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason:
            "'上一个'=assistant[-1](document,2), '上上个'=assistant[-2](scripts,3); scripts 更多"
                .to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_hint: "scripts".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_scalar_defers_compare_paths_result_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"compare_paths","left":{"path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","kind":"file","size_bytes":123},"right":{"path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","kind":"file","size_bytes":456},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":-333,"left_newer":null,"same_content":false}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "比较 Cargo.toml 和 Cargo.lock 哪个更大，顺手用一句通俗话解释原因"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:compare_targets".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_hint: "Cargo.lock|Cargo.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
    assert!(
        has_observed_answer_candidates(&loop_state),
        "compare_paths should remain available as observed facts for synthesis"
    );
}

#[test]
fn quantity_comparison_does_not_force_direct_scalar_observed_answer() {
    let route = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "比较 Cargo.toml 和 Cargo.lock 哪个更大".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:compare_targets".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_hint: "Cargo.lock|Cargo.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    assert!(!super::route_prefers_direct_observed_answer_for_scalar(
        &route
    ));
}

#[test]
fn direct_answer_defers_git_status_dirty_worktree_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前仓库是否存在未提交的改动，用一句话返回结果".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_summarizes_git_repository_state_without_volatile_count() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前仓库是否存在未提交的改动，用一句话返回结果".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("git.branch=main git.worktree=dirty")
    );
}

#[test]
fn direct_answer_uses_structured_git_repository_state_for_any_language() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "現在のリポジトリに未コミットの変更があるか、一文で答えてください"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("git.branch=main git.worktree=dirty")
    );
}

#[test]
fn direct_answer_prefers_git_state_over_later_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "synthesize_answer",
        "是的，当前仓库有 8 个文件有未提交的改动。",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查当前仓库是否存在未提交的改动，用一句话返回结果".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("git.branch=main git.worktree=dirty")
    );
}

#[test]
fn direct_answer_summarizes_git_branch_and_dirty_state_in_english() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n  dev\n* main\n  release\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent:
            "show the current git branch, then say whether the worktree looks clean or mid-edit"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: Some(1),
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "show the current git branch, then say in one plain English sentence whether the worktree looks clean or mid-edit"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };

    assert_eq!(
        extract_direct_answer_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("git.branch=main git.worktree=dirty")
    );
}

#[test]
fn direct_answer_defers_git_log_release_note_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "read_file",
        "RustClaw is a local Rust agent runtime centered on clawd.",
    ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","field_path":"workspace.package.version","value_text":"0.1.7"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_3",
            "git_basic",
            "exit=0\n09342a6a fix: expose nl execution and locator flows\n336e8d92 docs: update planner-first architecture diagrams\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Write a short release note for RustClaw.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            locator_hint: "RustClaw".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_scalar_extracts_git_commit_subject_from_oneline_log() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n09342a6a fix: expose nl execution and locator flows\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "return the latest git commit subject only".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::GitCommitSubject,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        Some("fix: expose nl execution and locator flows".to_string())
    );
}

#[test]
fn direct_answer_defers_git_status_clean_when_exit_only_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "git_basic", "exit=0\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "看看这个仓库现在有没有未提交改动，用一句话告诉我".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_git_status_dirty_without_branch_header_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        " M Cargo.toml\n?? new_file.txt\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "看看这个仓库现在有没有未提交改动，用一句话告诉我".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_preserves_run_cmd_directory_entry_names() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_output_test_{}_run_cmd_names",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "act_plan.log\nclawd.log\nfeishud.log\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下前 5 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("act_plan.log\nclawd.log\nfeishud.log")
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_preserves_run_cmd_semantic_directory_path_list() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            ".\n./scripts\n./scripts/nl_tests\n./crates/skills/browser_web/node_modules/playwright-core/bin\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "查找当前工作目录中哪些文件夹存放了 .sh 脚本文件，列出这些文件夹的名称"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::DirectoryNames,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some("/home/guagua/rustclaw".to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
            extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
                .as_deref(),
            Some(
                ".\n./scripts\n./scripts/nl_tests\n./crates/skills/browser_web/node_modules/playwright-core/bin"
            )
        );
}

#[test]
fn direct_answer_preserves_run_cmd_directory_entry_names_without_request_text_limit() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_output_test_{}_run_cmd_limit",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "a\nb\nc\nd\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "列出 logs 目录下前 2 个文件名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "logs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(temp_dir.to_string_lossy().to_string()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("a\nb\nc\nd")
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_formats_run_cmd_exists_probe_with_resolved_path() {
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd_observed_output_test_{}_run_cmd_exists",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&temp_dir);
    let file_path = temp_dir.join("rustclaw.service");
    std::fs::write(&file_path, "unit").expect("write fixture file");
    let resolved = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.clone())
        .to_string_lossy()
        .to_string();
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "EXISTS\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        auto_locator_path: Some(resolved.clone()),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(format!("有，路径：{resolved}").as_str())
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn direct_answer_formats_run_cmd_not_found_probe_as_no() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "NOT_FOUND\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "rustclaw.service".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("没有")
    );
}

#[test]
fn direct_answer_defers_health_check_json_for_act_free_shape() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "做一次 health check".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_formats_health_check_service_status_contract_without_llm() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0}}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "检查 clawd 服务当前状态，并用一句话说明来源。".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("service_status should use structured health_check evidence directly");

    assert!(answer.contains("health_check"));
    assert!(answer.contains("clawd_process_count=1"));
    assert!(answer.contains("clawd_health_port_open=true"));
    assert!(!answer.contains(r#""clawd_process_count""#));
}

#[test]
fn direct_answer_formats_wrapped_health_check_service_status_free_shape() {
    let mut loop_state = LoopState::new(2);
    let body = serde_json::json!({
        "extra": {
            "clawd_health_port_open": true,
            "clawd_log": {
                "exists": true,
                "keyword_error_count": 43
            },
            "clawd_process_count": 1,
            "system_health": {
                "os_family": "linux",
                "warnings": ["disk_root_low"]
            },
            "telegramd_log": {
                "exists": true,
                "keyword_error_count": 1
            },
            "telegramd_process_count": 0
        },
        "text": "{\"clawd_health_port_open\":true,\"clawd_process_count\":1}"
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", &body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "Show system/service status".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some("show status".to_string()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("wrapped health_check evidence should provide service status directly");

    assert!(answer.contains("health_check"));
    assert!(answer.contains("clawd_process_count=1"));
    assert!(answer.contains("clawd_health_port_open=true"));
    assert!(!answer.contains("unclear"));
    assert!(!answer.contains(r#""extra""#));
}

#[test]
fn direct_answer_defers_health_check_diagnostic_summary_for_system_health_fields() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":43},"system_health":{"os_family":"linux","load_avg_1m":3.81,"memory_available_bytes":11270471680,"disk_root_available_bytes":18108059648,"warnings":[]}}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "执行基础健康检查，列出最重要的诊断结论".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("service_status health_check should expose diagnostic machine fields directly");
    assert!(answer.contains("health_check.summary"));
    assert!(answer.contains("clawd.status=running"));
    assert!(answer.contains("clawd_process_count=1"));
    assert!(answer.contains("clawd_health_port_open=true"));
    assert!(answer.contains("clawd_log.keyword_error_count=43"));
    assert!(answer.contains("system_health.load_avg_1m=3.81"));
    assert!(answer.contains("system_health.memory_available_bytes=11270471680"));
    assert!(answer.contains("system_health.disk_root_available_bytes=18108059648"));
}

#[test]
fn direct_answer_defers_health_check_summary_for_act_free_shape() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_process_count":7,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "对系统做一次基础健康检查，只总结操作系统信息，RustClaw 自身不展开总结，仅返回其关键字段"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_passes_health_check_json_only_for_raw_output_contract() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"clawd_health_port_open":true,"telegramd_process_count":0}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "health_check", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "run health_check and return the raw output".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(body)
    );
}

#[test]
fn direct_answer_defers_health_check_summary_over_later_steps_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":12,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "system_basic",
        r#"{"action":"info","os":"macos","hostname":"example"}"#,
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "Run a basic health check. Summarize only the host operating system, and for RustClaw itself just list the key fields.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: OutputLocatorKind::None,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_health_check_one_sentence_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_health_check_unhealthy_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":0,"telegramd_process_count":1,"clawd_health_port_open":false,"clawd_log":{"exists":true,"keyword_error_count":3},"telegramd_log":{"exists":true,"keyword_error_count":0}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "run a basic health check here and summarize only the most important findings"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_health_check_telegramd_stopped_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_health_check_language_sensitive_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "帮我做一次基础健康检查，只列最重要的结论".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some(
            "run a basic health check here and summarize only the most important findings"
                .to_string(),
        ),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_health_check_os_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":12,"telegramd_process_count":0,"clawd_health_port_open":false,"clawd_log":{"exists":false},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "做一次基础健康检查，只返回操作系统层面的关键字段，不要包含 RustClaw 自身的状态摘要"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_failed_safe_clarify".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some(
            "做一次基础健康检查，只总结操作系统；RustClaw 自身不要总结，直接给我关键字段。"
                .to_string(),
        ),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_health_check_os_warning_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":1,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":true,"keyword_error_count":0},"system_health":{"os_family":"linux","warnings":["disk_root_low"]}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "run a basic health check here and summarize only the most important findings"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        user_request: Some(
            "run a basic health check here and summarize only the most important findings"
                .to_string(),
        ),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_process_basic_port_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "process_basic",
            "exit=0\nCOMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nclawd 4498 testuser 12u IPv4 0x0 0t0 TCP *:8787 (LISTEN)\nnginx 51129 testuser 6u IPv4 0x0 0t0 TCP *:80 (LISTEN)\nss-local 424 testuser 6u IPv4 0x0 0t0 TCP 127.0.0.1:1086 (LISTEN)\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "看看这台机器现在有哪些端口在监听，然后挑最值得注意的几个简单说一下"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_process_basic_port_status_contract_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "process_basic",
            "exit=0\nState  Recv-Q Send-Q Local Address:Port  Peer Address:PortProcess\nLISTEN 0      4096   127.0.0.53%lo:53         0.0.0.0:*\nLISTEN 0      4096         0.0.0.0:8787       0.0.0.0:*    users:((\"clawd\",pid=706551,fd=31))\nLISTEN 0      4096         0.0.0.0:22         0.0.0.0:*\nLISTEN 0      511          0.0.0.0:80         0.0.0.0:*\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "查看当前机器监听的端口，列出最值得注意的端口并简单说明".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_formats_process_basic_service_status_contract_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "process_basic",
        "exit=0\nPID PPID %CPU %MEM COMM\n413590 7620 1.0 0.2 clawd",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "检查 clawd 服务当前状态，并用一句话说明来源。".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("service_status should use process_basic table evidence directly");

    assert!(answer.contains("process_basic"));
    assert!(answer.contains("COMM=clawd"));
    assert!(!answer.contains("PID PPID"));
}

#[test]
fn direct_answer_formats_process_basic_multi_row_cpu_inventory() {
    let answer = super::process_basic_service_status_direct_answer_candidate(
        None,
        "exit=0\nPID PPID %CPU %MEM COMM\n1713539 8057 6.4 2.7 WebKitWebProces\n8923 7620 6.1 0.3 ptyxis\n7886 7620 3.5 1.8 gnome-shell\n9127 9116 3.5 4.2 codex\n1100416 83086 1.2 1.7 chrome",
        Some(OutputResponseShape::OneSentence),
        false,
    )
    .expect("multi-row process inventory should produce a data-grounded summary");

    assert!(answer.contains("WebKitWebProces"));
    assert!(answer.contains("ptyxis"));
    assert!(answer.contains("gnome-shell"));
    assert!(answer.contains("codex"));
    assert!(answer.contains("chrome"));
    assert!(answer.contains("6.4"));
    assert!(answer.contains("最值得注意的是 WebKitWebProces"));
    assert!(!answer.contains("PID PPID"));
}

#[test]
fn direct_answer_formats_process_basic_no_match_as_not_running() {
    let answer = super::process_basic_service_status_direct_answer_candidate(
        None,
        "exit=0\nPID PPID %CPU %MEM COMM\nno matching processes for filter: telegramd",
        Some(OutputResponseShape::OneSentence),
        true,
    )
    .expect("no-match process output should produce a status answer");

    assert!(answer.contains("telegramd"));
    assert!(answer.contains("not running"));
    assert!(!answer.contains("1 process record"));
    assert!(!answer.contains("COMM=telegramd"));
}

#[test]
fn direct_answer_defers_http_basic_one_sentence_summary_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "http_basic",
        "status=200\n{\"ok\":true}\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "请求一下 http://127.0.0.1:8787/v1/health ，如果能通就简短总结结果"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Url,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_preserves_http_basic_raw_scalar_for_free_shape() {
    let mut loop_state = LoopState::new(2);
    let body = "status=200\n{\"ok\":true}\n";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "http_basic", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "请求接口并返回原始结果".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Url,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("status=200")
    );
}

#[test]
fn direct_answer_defers_http_basic_web_page_summary_to_observed_synthesis() {
    let mut loop_state = LoopState::new(2);
    let body =
        "status=200\n{\"ok\":true,\"data\":{\"version\":\"0.1.7\",\"worker_state\":\"running\"}}\n";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "http_basic", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "web_page_summary".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Url,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::WebPageSummary,
            locator_hint: "http://127.0.0.1:8787/v1/health".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_http_basic_url_service_status_to_observed_synthesis() {
    let mut loop_state = LoopState::new(2);
    let body = "status=200\n{\"ok\":true,\"data\":{\"version\":\"0.1.7\",\"worker_state\":\"running\",\"queue_length\":0,\"bound_channel_count\":3}}\n";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "http_basic", body));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "service_status".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_formats_service_control_status_summary_for_chinese_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=stopped","post_state":"telegramd=stopped","verified":true,"key_evidence":["telegramd process_count=0 memory_rss_bytes=Some(0)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=stopped"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "帮我检查 telegramd 现在是不是在运行，顺手简短解释状态".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: "telegramd".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("telegramd 当前状态是 `telegramd=stopped`：rustclaw 已完成检查，未显示为运行中。")
    );
}

#[test]
fn direct_answer_formats_service_control_status_summary_for_english_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "service_control",
            r#"{"status":"ok","service_name":"telegramd","manager_type":"rustclaw","requested_action":"status","executed_actions":["status"],"pre_state":"telegramd=running","post_state":"telegramd=running","verified":true,"key_evidence":["telegramd process_count=1 memory_rss_bytes=Some(1024)"],"failure_reason":"","next_step":"","summary":"Status: telegramd=running"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent:
            "check whether telegramd is running right now and briefly explain the status"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ServiceStatus,
            locator_hint: "telegramd".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("telegramd is running: rustclaw reports `telegramd=running` and verification passed.")
    );
}

#[test]
fn observed_entries_compact_log_analyze_json_into_summary() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "log_analyze",
            r#"{"path":"/tmp/test.log","total_lines":120,"keyword_counts":{"error":9,"panic":1},"recent_matches":["10: error one","20: panic two"]}"#,
        ));
    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("log_analyze path=/tmp/test.log total_lines=120"));
    assert!(entries[0].contains("keyword_counts: error=9, panic=1"));
    assert!(entries[0].contains("recent_matches:\n- 10: error one\n- 20: panic two"));
    assert!(!entries[0].contains(r#""keyword_counts""#));
}

#[test]
fn observed_answer_parser_strips_bare_json_language_prefix() {
    let raw = "json\n{\"answer\":\"ok\",\"qualified\":true}";
    assert_eq!(
        super::strip_bare_json_language_prefix(raw),
        "{\"answer\":\"ok\",\"qualified\":true}"
    );
    assert_eq!(
        super::strip_bare_json_language_prefix("json response follows"),
        "json response follows"
    );
}

#[test]
fn observed_answer_parser_unwraps_nested_finalizer_envelope() {
    let raw = "json\n{\"answer\":\"# RustClaw\\n正文\",\"qualified\":true,\"needs_clarify\":false,\"is_meta_instruction\":false,\"publishable\":true,\"confidence\":0.85,\"reason\":\"grounded\"}";
    assert_eq!(
        super::extract_answer_from_finalizer_envelope_text(raw).as_deref(),
        Some("# RustClaw\n正文")
    );
}

/// §D2.b：finalizer_out schema 与 `ObservedAnswerFallbackOut` 漂移检查。
///
/// 校验内容：
/// 1. `prompts/schemas/finalizer_out.schema.json` 是合法 JSON 且为 object schema；
/// 2. `properties` ⊇ `ObservedAnswerFallbackOut` 全部字段（含 serde rename 后的 `reason`）；
/// 3. `required` 列表精确包含 5 个核心硬要求字段（answer + 4 个布尔 + confidence）；
/// 4. 完整性闭环：把一份 schema-conformant 的最小负载 round-trip
///    `serde_json::from_str::<ObservedAnswerFallbackOut>` 必须成功，且 confidence 0/1
///    边界都被接受。
///
/// 任意不满足说明 prompt / schema / parser 三者已漂移，build 红灯。
#[test]
fn finalizer_out_schema_drift() {
    const SCHEMA_RAW: &str = include_str!("../../../../prompts/schemas/finalizer_out.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("finalizer_out.schema.json must be valid JSON");
    assert_eq!(
        schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "schema root must be object"
    );

    const STRUCT_FIELDS: &[&str] = &[
        "answer",
        "qualified",
        "needs_clarify",
        "is_meta_instruction",
        "publishable",
        "confidence",
        "reason",
    ];
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema must have `properties` object");
    for field in STRUCT_FIELDS {
        assert!(
                properties.contains_key(*field),
                "schema missing parser field `{}` under properties — sync prompts/schemas/finalizer_out.schema.json with ObservedAnswerFallbackOut",
                field
            );
    }

    let required: std::collections::HashSet<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("schema must have `required`")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    let expected_required: std::collections::HashSet<&str> = [
        "answer",
        "qualified",
        "needs_clarify",
        "is_meta_instruction",
        "publishable",
        "confidence",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        required, expected_required,
        "finalizer_out required set drifted from canonical 5+1"
    );

    // 步骤 4：最小 schema-conformant 负载必须能解码到 parser struct。
    let probes: &[(&str, &str)] = &[
        (
            "minimum",
            r#"{"answer":"ok","qualified":true,"needs_clarify":false,"is_meta_instruction":false,"publishable":true,"confidence":0.0}"#,
        ),
        (
            "boundary_high",
            r#"{"answer":"ok","qualified":true,"needs_clarify":false,"is_meta_instruction":false,"publishable":true,"confidence":1.0,"reason":"r"}"#,
        ),
        (
            "needs_clarify_with_empty_answer",
            r#"{"answer":"","qualified":false,"needs_clarify":true,"is_meta_instruction":false,"publishable":false,"confidence":0.5}"#,
        ),
    ];
    for (label, raw) in probes {
        serde_json::from_str::<super::ObservedAnswerFallbackOut>(raw).unwrap_or_else(|err| {
            panic!(
                "ObservedAnswerFallbackOut probe `{}` failed: {} (raw: {})",
                label, err, raw
            )
        });
    }
}

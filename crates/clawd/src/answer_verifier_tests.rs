use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;

use super::{
    answer_verifier_user_request_for_prompt, backend_identity_metadata_answer_verifier_guard,
    execution_evidence_prompt_block, local_compound_listing_answer_verifier_gap,
    local_missing_evidence_verifier_gap, local_missing_evidence_verifier_gap_for_answer,
    observed_scalar_values_from_evidence_map, observed_scalar_values_from_evidence_map_for_route,
    observed_single_path_values_from_evidence_map, observed_strict_list_items_from_evidence_map,
    observed_strict_list_items_from_evidence_map_for_route, observed_table_cells_from_evidence_map,
    output_contract_prompt_block, recent_structured_scalar_values_from_journal,
    route_contract_marker_is_scalar_path_only, should_verify_answer,
    strict_list_route_allows_observed_subset, structural_satisfaction_can_skip_verifier,
    structurally_satisfies_answer_contract, AnswerVerifierOut,
};
use super::{health_check_value_from_output, observed_find_ext_results};

fn route_with_mode(ask_mode: crate::AskMode) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode,
        resolved_intent: "test intent".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "test".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    }
}

#[test]
fn verifier_contract_markers_do_not_require_semantic_enum() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    route.route_reason = "file_paths".to_string();
    assert!(strict_list_route_allows_observed_subset(&route));

    route.route_reason = "scalar_path_only".to_string();
    assert!(route_contract_marker_is_scalar_path_only(&route));
}

fn state_with_mimo_provider() -> crate::AppState {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.llm_providers = vec![Arc::new(crate::LlmProviderRuntime {
        config: claw_core::config::LlmProviderConfig {
            name: "vendor-mimo".to_string(),
            provider_type: "openai_compat".to_string(),
            base_url: "http://fixture.invalid".to_string(),
            api_key: "fixture".to_string(),
            model: "mimo-v2.5-pro".to_string(),
            context_window_tokens: None,
            priority: 1,
            timeout_seconds: 5,
            max_concurrency: 1,
            params: claw_core::config::LlmProviderParams::default(),
        },
        client: reqwest::Client::new(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    })];
    state
}

#[test]
fn answer_verifier_prompt_request_preserves_original_language_over_resolved_intent() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-verifier-language-source".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({
            "text": "用合适的只读文件读取能力查看 README.md 前 20 行，只回答文件是否存在、读取到的行数，以及标题中是否出现 RustClaw；不要用 shell cat 兜底。"
        })
        .to_string(),
    };
    let resolved = "Use a read-only file reading capability to read README.md head 20 lines.";

    let request_for_prompt = answer_verifier_user_request_for_prompt(&task, resolved);
    let language_hint =
        crate::language_policy::task_response_language_hint(&state, &task, resolved);

    assert_eq!(language_hint, "zh-CN");
    assert!(request_for_prompt.contains("Original user request:"));
    assert!(request_for_prompt.contains("只回答文件是否存在"));
    assert!(request_for_prompt.contains("Resolved semantic request:"));
    assert!(request_for_prompt.contains("Use a read-only file reading capability"));
    assert!(request_for_prompt.contains("preserve the original user's language"));
}

fn backend_identity_guard_route() -> crate::RouteResult {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.route_reason =
        "agent_display_name_hint_backend_metadata_removed; pure_chat_agent_loop_submode"
            .to_string();
    route
}

#[test]
fn backend_identity_metadata_guard_accepts_runtime_identity_label() {
    let state = state_with_mimo_provider();
    let route = backend_identity_guard_route();

    let guard = backend_identity_metadata_answer_verifier_guard(&state, &route, "RustClaw")
        .expect("runtime identity should be handled without model verifier");

    assert!(guard.pass);
    assert!(guard.missing_evidence_fields.is_empty());
    assert!(!guard.should_retry);
}

#[test]
fn backend_identity_metadata_guard_rejects_provider_identity_leak() {
    let state = state_with_mimo_provider();
    let route = backend_identity_guard_route();

    let guard = backend_identity_metadata_answer_verifier_guard(
        &state,
        &route,
        "你好，我是 MiMo-v2.5-pro，由小米 MiMo 团队开发。",
    )
    .expect("backend provider identity should be handled structurally");

    assert!(!guard.pass);
    assert_eq!(guard.missing_evidence_fields, vec!["identity"]);
    assert_eq!(
        guard.answer_incomplete_reason,
        "backend_identity_metadata_in_final_answer"
    );
    assert!(guard.should_retry);
}

#[test]
fn backend_identity_metadata_guard_requires_route_marker() {
    let state = state_with_mimo_provider();
    let route = route_with_mode(crate::AskMode::planner_execute_plain());

    assert!(
        backend_identity_metadata_answer_verifier_guard(&state, &route, "MiMo-v2.5-pro",).is_none()
    );
}

#[test]
fn answer_verifier_schema_accepts_typed_output() {
    let raw = json!({
        "pass": false,
        "missing_evidence_fields": ["size_bytes"],
        "answer_incomplete_reason": "missing requested size evidence",
        "should_retry": true,
        "retry_instruction": "Collect file metadata and answer with path plus size.",
        "confidence": 0.86
    });
    let validated = crate::prompt_utils::validate_against_schema::<AnswerVerifierOut>(
        &raw.to_string(),
        crate::prompt_utils::PromptSchemaId::AnswerVerifier,
    )
    .expect("schema should validate answer verifier output");
    assert!(!validated.value.pass);
    assert!(validated.value.should_retry);
}

#[test]
fn answer_verifier_schema_drift() {
    const SCHEMA_RAW: &str = include_str!("../../../prompts/schemas/answer_verifier.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("answer_verifier schema must be valid JSON");
    assert_eq!(
        schema.get("type").and_then(serde_json::Value::as_str),
        Some("object"),
        "answer_verifier schema root must be object"
    );
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&json!(false)),
        "answer_verifier schema must reject unknown fields after canonicalization"
    );

    let expected = [
        "pass",
        "missing_evidence_fields",
        "answer_incomplete_reason",
        "should_retry",
        "retry_instruction",
        "confidence",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let properties = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .expect("schema must have object properties");
    let actual = properties
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "answer_verifier.schema.json properties drifted from AnswerVerifierOut"
    );

    let required = schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .expect("schema must have required fields")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        required, expected,
        "answer_verifier.schema.json required set drifted from AnswerVerifierOut"
    );

    let raw = json!({
        "pass": true,
        "missing_evidence_fields": [],
        "answer_incomplete_reason": "",
        "should_retry": false,
        "retry_instruction": "",
        "confidence": 1.0
    })
    .to_string();
    crate::prompt_utils::validate_against_schema::<AnswerVerifierOut>(
        &raw,
        crate::prompt_utils::PromptSchemaId::AnswerVerifier,
    )
    .expect("schema-conformant answer verifier payload must deserialize");
}

#[test]
fn answer_verifier_prompt_preserves_compound_deliverables_on_retry() {
    const PROMPT_RAW: &str =
        include_str!("../../../prompts/layers/overlays/answer_verifier_prompt.md");
    assert!(PROMPT_RAW.contains("preserve the already required deliverable"));
    assert!(PROMPT_RAW.contains("combined final answer"));
    assert!(PROMPT_RAW.contains("include the observed listed items and the synthesis"));
}

#[test]
fn answer_verifier_output_contract_exposes_evidence_profile() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.requires_content_evidence = true;

    let block = output_contract_prompt_block(&route);

    assert!(block.contains("\"contract_matrix\""));
    assert!(block.contains("\"compact_line\""));
    assert!(block.contains("\"evidence_profile\""));
    assert!(block.contains("\"workspace_user_docs_first\""));
    assert!(!block.contains("\"observation_extractors\""));
    assert!(!block.contains("\"observation_sources\""));
}

#[test]
fn answer_verifier_gap_is_high_confidence_only() {
    let low = AnswerVerifierOut {
        pass: false,
        confidence: 0.2,
        ..AnswerVerifierOut::default()
    };
    let high = AnswerVerifierOut {
        pass: false,
        confidence: 0.8,
        ..AnswerVerifierOut::default()
    };
    assert!(!low.high_confidence_gap());
    assert!(high.high_confidence_gap());
}

#[test]
fn answer_verifier_gap_respects_explicit_retry_flag() {
    let retry = AnswerVerifierOut {
        pass: false,
        should_retry: true,
        answer_incomplete_reason: "answer omitted requested synthesis".to_string(),
        confidence: 0.2,
        ..AnswerVerifierOut::default()
    };

    assert!(retry.high_confidence_gap());
}

#[test]
fn answer_verifier_normalizes_high_confidence_gap_to_retry() {
    let normalized = AnswerVerifierOut {
        pass: false,
        confidence: 0.82,
        retry_instruction: "  ".to_string(),
        ..AnswerVerifierOut::default()
    }
    .normalized();
    assert!(normalized.should_retry);
    assert!(!normalized.retry_instruction.trim().is_empty());
}

#[test]
fn execution_evidence_prompt_uses_provider_safe_redacted_view() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-provider-safe", "ask", "检查配置");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "path": "/tmp/app.toml",
                "token": "sk-test-secret-token-that-should-not-leak"
            })
            .to_string(),
        ),
        error: Some("password=secret-value-that-should-not-leak".to_string()),
        started_at: 1,
        finished_at: 2,
    });

    let block = execution_evidence_prompt_block(&journal);

    assert!(block.contains("\"observed_evidence\""));
    assert!(block.contains("\"output_excerpt_hash\""));
    assert!(block.contains("\"error_excerpt_hash\""));
    assert!(!block.contains("\"output_excerpt\""));
    assert!(!block.contains("\"error_excerpt\""));
    assert!(!block.contains("sk-test-secret-token-that-should-not-leak"));
    assert!(!block.contains("password=secret-value-that-should-not-leak"));
    assert!(block.contains("\"redacted\": true"));
    assert!(block.contains("\"provider_evidence_view\": \"provider_safe_redacted\""));
    assert!(block.contains("\"raw_excerpt_policy\": \"no_full_raw_excerpt\""));
}

#[test]
fn execution_evidence_prompt_excludes_prior_synthesis_candidates() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-provider-safe-observations-only",
        "ask",
        "list recent logs",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["model_io.log", "act_plan.log"],
                "entries": [
                    {"name": "model_io.log", "modified_ts": 1780028593, "size_bytes": 143376979},
                    {"name": "act_plan.log", "modified_ts": 1780028552, "size_bytes": 5347833}
                ],
                "sort_by": "mtime_desc"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "synthesize_answer".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            "The two most recent files are model_io.log.2026-05-28 and model_io.log.2026-05-27."
                .to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let block = execution_evidence_prompt_block(&journal);

    assert!(block.contains("model_io.log"));
    assert!(block.contains("act_plan.log"));
    assert!(!block.contains("model_io.log.2026-05-28"));
    assert!(!block.contains("model_io.log.2026-05-27"));
    assert!(!block.contains("synthesize_answer"));
}

#[test]
fn execution_evidence_prompt_includes_error_step_observed_evidence() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-provider-safe-error-observation",
        "ask",
        "run commands and summarize success and failure",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("/home/guagua/rustclaw\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "platform": "linux",
            "extra": {
                "command": "definitely_missing_command_rustclaw_english_67890",
                "exit_code": 127,
                "exit_category": "command_not_found",
                "stderr": "bash: line 1: definitely_missing_command_rustclaw_english_67890: command not found\n",
                "output_truncated": false
            }
        })
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(err),
        started_at: 3,
        finished_at: 4,
    });

    let block = execution_evidence_prompt_block(&journal);

    assert!(block.contains(r#""step_id": "step_2""#), "block: {block}");
    assert!(block.contains(r#""status": "error""#), "block: {block}");
    assert!(
        block.contains(r#""field": "command_output""#),
        "block: {block}"
    );
    assert!(block.contains(r#""field": "exit_code""#), "block: {block}");
    assert!(
        !block.contains("definitely_missing_command_rustclaw_english_67890"),
        "block: {block}"
    );
}

#[test]
fn execution_failed_step_answer_uses_failed_machine_tokens_not_success_stdout() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.requires_content_evidence = true;

    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-failed-step-structural",
        "ask",
        "run two commands and report only the failed step",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("RC_RENDER_OK\n".to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 127",
            "platform": "linux",
            "extra": {
                "command": "definitely_missing_command_rustclaw_render_ko_0605",
                "exit_code": 127,
                "exit_category": "command_not_found",
                "stderr": "bash: line 1: definitely_missing_command_rustclaw_render_ko_0605: command not found\n",
                "output_truncated": false
            }
        })
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(err),
        started_at: 3,
        finished_at: 4,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_3".to_string(),
        skill: "synthesize_answer".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some("RC_RENDER_OK".to_string()),
        error: None,
        started_at: 5,
        finished_at: 6,
    });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "step_2: definitely_missing_command_rustclaw_render_ko_0605 failed with exit code 127",
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "RC_RENDER_OK",
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        r#"{"message_key":"clawd.msg.execution.failed_step_status"}"#,
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "completed",
    ));
}

#[test]
fn execution_evidence_prompt_includes_compact_numeric_evidence() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-provider-safe-size", "ask", "size?");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "count_inventory",
                "counts": {
                    "dirs": 7,
                    "files": 11,
                    "total": 18,
                    "total_size_bytes": 57264444014_u64
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let block = execution_evidence_prompt_block(&journal);

    assert!(block.contains("\"key_numeric_evidence\""));
    assert!(block.contains("\"counts.total_size_bytes\""));
    assert!(block.contains("57264444014"));
    assert!(!block.contains("\"output_excerpt\""));
}

#[test]
fn direct_answer_route_skips_answer_verifier() {
    let route = route_with_mode(crate::AskMode::direct_answer());
    let journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
    assert!(!should_verify_answer(&route, &journal, "hi"));
}

#[test]
fn pure_chat_agent_loop_submode_skips_answer_verifier_for_freeform_response() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.route_reason = "pure_chat_agent_loop_submode".to_string();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.wants_file_delivery = false;
    let journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "direct response request");

    assert!(!should_verify_answer(
        &route,
        &journal,
        "candidate response"
    ));
}

#[test]
fn pure_chat_agent_loop_submode_skips_answer_verifier_after_terminal_respond_step() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.route_reason = "pure_chat_agent_loop_submode".to_string();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.wants_file_delivery = false;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "direct response request");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "respond",
            "candidate response",
        ));

    assert!(!should_verify_answer(
        &route,
        &journal,
        "candidate response"
    ));
}

#[test]
fn pure_chat_agent_loop_backend_identity_marker_still_uses_answer_verifier() {
    let mut route = backend_identity_guard_route();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.wants_file_delivery = false;
    let journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "identity response request");

    assert!(should_verify_answer(&route, &journal, "candidate response"));
}

#[test]
fn tool_discovery_context_only_route_skips_answer_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.route_reason = "tool_discovery".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ToolDiscovery;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let journal =
        crate::task_journal::TaskJournal::for_task("task-1", "ask", "current capabilities");

    assert!(!should_verify_answer(
        &route,
        &journal,
        "`fs_basic`, `git_basic`, `weather`"
    ));
}

#[test]
fn non_tool_discovery_contract_marker_still_uses_answer_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.route_reason = "content_excerpt_summary".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "summarize");

    assert!(should_verify_answer(&route, &journal, "summary"));
}

#[test]
fn clarify_final_status_skips_answer_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.requires_content_evidence = true;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

    assert!(!should_verify_answer(
        &route,
        &journal,
        "please provide the missing path"
    ));
}

#[test]
fn local_missing_evidence_gap_reports_required_fields() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap", "ask", "exists?");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"path": "/tmp/a.txt", "exists": true}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let gap = local_missing_evidence_verifier_gap(&route, &journal).expect("missing kind evidence");
    assert_eq!(gap.missing_evidence_fields, vec!["kind"]);
    assert!(gap.should_retry);
    assert!(gap.high_confidence_gap());
}

#[test]
fn local_missing_evidence_gap_skips_when_required_fields_are_observed() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap-ok", "ask", "list names");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"names": ["Cargo.toml"]}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
}

#[test]
fn config_guard_machine_payload_skips_missing_evidence_verifier_gap() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config.toml".to_string();
    let answer = json!({
        "message_key": "clawd.msg.config_edit.guard",
        "reason_code": "config_edit_guard_risk_found",
        "path": "configs/config.toml",
        "risk_count": 2,
        "count": 2,
        "candidates": [
            "tools.allow_sudo=true",
            "tools.allow_path_outside_workspace=true"
        ]
    })
    .to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-config-guard-json", "ask", "config");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "guard_config",
                "path": "configs/config.toml",
                "risk_count": 2,
                "candidates": [
                    "tools.allow_sudo=true",
                    "tools.allow_path_outside_workspace=true"
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });

    assert!(local_missing_evidence_verifier_gap_for_answer(&route, &journal, &answer).is_none());
    assert!(structural_satisfaction_can_skip_verifier(
        &route, &journal, &answer
    ));
}

#[test]
fn local_missing_evidence_gap_does_not_block_on_negative_evidence_only() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-negative-evidence-only",
        "ask",
        "current workspace path",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [{
                    "path": ".",
                    "exists": false,
                    "kind": "missing"
                }],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = crate::task_journal::evidence_coverage_for_route(&route, &journal);
    assert_eq!(
        coverage.missing_evidence,
        vec!["negative_evidence(exists_false)"]
    );
    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
}

#[test]
fn local_missing_evidence_gap_skips_structured_not_found_terminal_finalizer() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "definitely_missing_dir_rustclaw_xyz/".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap-not-found", "ask", "list");
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(crate::skills::structured_skill_error_from_parts(
            "fs_basic",
            "not_found",
            "target not found",
            Some("linux"),
            Some(json!({
                "operation": "list_dir",
                "path": "definitely_missing_dir_rustclaw_xyz/"
            })),
        )),
        started_at: 1,
        finished_at: 2,
    });
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });

    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
}

#[test]
fn structural_satisfaction_skips_latest_not_found_answer_with_same_path() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/rustclaw-missing.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-gap-not-found-answer",
        "ask",
        "read missing path",
    );
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/rustclaw-missing.md".to_string()),
        started_at: 1,
        finished_at: 2,
    });

    assert!(structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "/tmp/rustclaw-missing.md"
    ));
}

#[test]
fn structural_satisfaction_skips_successful_stat_missing_answer_with_same_path() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/rustclaw-missing.md | README.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-gap-stat-missing-answer",
        "ask",
        "stat missing path and summarize readme",
    );
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "path_batch_facts",
                "count": 1,
                "facts": [
                    {
                        "exists": false,
                        "kind": "missing",
                        "path": "/tmp/rustclaw-missing.md"
                    }
                ],
                "include_missing": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    assert!(structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "exists=false path=/tmp/rustclaw-missing.md kind=missing"
    ));
}

#[test]
fn structural_satisfaction_keeps_verifier_when_not_found_answer_omits_path() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_hint = "/tmp/rustclaw-missing.md".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-local-gap-not-found-answer-no-path",
        "ask",
        "read missing path",
    );
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/rustclaw-missing.md".to_string()),
        started_at: 1,
        finished_at: 2,
    });

    assert!(!structural_satisfaction_can_skip_verifier(
        &route, &journal, "missing"
    ));
}

#[test]
fn should_verify_answer_skips_permission_denied_terminal_finalizer() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/etc/shadow".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-terminal-permission", "ask", "read");
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(crate::skills::structured_skill_error_from_parts(
            "system_basic",
            "permission_denied",
            "read operation failed",
            Some("linux"),
            Some(json!({
                "operation": "read_file",
                "path": "/etc/shadow"
            })),
        )),
        started_at: 1,
        finished_at: 2,
    });
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);

    assert!(!should_verify_answer(
        &route,
        &journal,
        "message_key=content_permission_denied path=/etc/shadow"
    ));
}

#[test]
fn local_missing_evidence_gap_skips_crypto_account_access_terminal_finalizer() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::MarketQuote;
    route.output_contract.requires_content_evidence = true;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap-crypto", "ask", "positions");
    journal.record_route_result(&route);
    let marker = r#"__RC_CRYPTO_ACCOUNT_ACCESS_ERROR__:{"exchange":"binance","detail":"binance error status=401: {\"code\":-2015,\"msg\":\"Invalid API-key, IP, or permissions for action.\"}"}"#;
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "crypto".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "crypto",
                "error_kind": "unknown",
                "error_text": marker,
                "extra": null
            })
        )),
        started_at: 1,
        finished_at: 2,
    });
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });

    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
}

#[test]
fn local_missing_evidence_gap_keeps_gap_for_non_missing_terminal_error() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "maybe_dir/".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-local-gap-error", "ask", "list");
    journal.record_route_result(&route);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(crate::skills::structured_skill_error_from_parts(
            "fs_basic",
            "invalid_args",
            "invalid list arguments",
            Some("linux"),
            Some(json!({
                "operation": "list_dir",
                "path": "maybe_dir/"
            })),
        )),
        started_at: 1,
        finished_at: 2,
    });
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });

    let gap = local_missing_evidence_verifier_gap(&route, &journal).expect("gap should remain");

    assert_eq!(gap.missing_evidence_fields, vec!["candidates"]);
}

#[test]
fn local_compound_listing_gap_rejects_answer_that_drops_observed_names() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.resolved_intent = "selector_limit=3; summarize listed content".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-compound-list", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["archive", "release_checklist.md", "service_notes.md"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Release Checklist\n3|1. Verify configuration loads correctly."
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let gap = local_compound_listing_answer_verifier_gap(
        &route,
        &journal,
        "release_checklist.md is an operator checklist.",
    )
    .expect("compound listing gap");

    assert_eq!(gap.missing_evidence_fields, vec!["candidates"]);
    assert!(gap.answer_incomplete_reason.contains("archive"));
    assert!(gap.should_retry);
}

#[test]
fn local_compound_listing_gap_accepts_answer_with_observed_names() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "selector_limit=3; summarize listed content".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-compound-list-ok", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["archive", "release_checklist.md", "service_notes.md"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Release Checklist\n3|1. Verify configuration loads correctly."
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    assert!(local_compound_listing_answer_verifier_gap(
        &route,
        &journal,
        "archive, release_checklist.md, and service_notes.md are listed, and release_checklist.md is an operator checklist."
    )
    .is_none());
}

#[test]
fn local_compound_listing_gap_applies_to_directory_purpose_summary() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "selector_limit=3; summarize purpose".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-gap", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["alpha.md", "beta.json", "notes.txt"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Alpha\n2|setup notes"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let gap = local_compound_listing_answer_verifier_gap(
        &route,
        &journal,
        "alpha.md and notes.txt look documentation-oriented.",
    )
    .expect("directory purpose summary should require observed listing items");

    assert_eq!(gap.missing_evidence_fields, vec!["candidates"]);
    assert!(gap.answer_incomplete_reason.contains("beta.json"));
}

#[test]
fn directory_purpose_summary_structurally_satisfies_listing_content_answer() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "selector_limit=3; summarize purpose".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-ok", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["alpha.md", "beta.json", "notes.txt"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "document/alpha.md",
                "excerpt": "1|# Alpha\n2|setup notes"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "alpha.md, beta.json, and notes.txt are listed; based on the observed excerpt, this looks documentation-oriented."
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "alpha.md and notes.txt are listed; based on the observed excerpt, this looks documentation-oriented."
    ));
}

#[test]
fn compound_listing_gap_respects_requested_numeric_limit() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "selector_limit=2; summarize purpose".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-limit", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["alpha.md", "beta.json", "notes.txt"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "excerpt": "1|# Alpha\n2|setup notes"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer =
        "alpha.md and beta.json are listed; based on the observed excerpt, this looks documented.";

    assert!(local_compound_listing_answer_verifier_gap(&route, &journal, answer).is_none());
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
}

#[test]
fn directory_purpose_summary_line_count_number_is_not_listing_limit() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "summarize directory and keep answer within 5 lines".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-line-count", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": [
                    "answer_verifier.schema.json",
                    "contract_repair_judge.schema.json",
                    "delivery_text_classifier.schema.json",
                    "intent_normalizer.schema.json"
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "prompts/schemas/intent_normalizer.schema.json",
                "excerpt": "1|title IntentNormalizerOut"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer = "intent_normalizer.schema.json; content_excerpt=IntentNormalizerOut";

    assert!(local_compound_listing_answer_verifier_gap(&route, &journal, answer).is_none());
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
}

#[test]
fn content_excerpt_summary_line_count_number_is_not_listing_limit() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent = "summarize observed content and keep answer within 5 lines".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-content-summary-line-count",
        "ask",
        "prompt",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": [
                    "answer_verifier.schema.json",
                    "contract_repair_judge.schema.json",
                    "delivery_text_classifier.schema.json",
                    "intent_normalizer.schema.json"
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "prompts/schemas/intent_normalizer.schema.json",
                "excerpt": "1|title IntentNormalizerOut"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer = "intent_normalizer.schema.json; content_excerpt=IntentNormalizerOut";

    assert!(local_compound_listing_answer_verifier_gap(&route, &journal, answer).is_none());
}

#[test]
fn unbounded_directory_purpose_summary_does_not_require_all_listing_names() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent =
        "summarize workspace organization from top-level directories".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-unbounded", "ask", "prompt");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names": ["UI", "configs", "crates", "scripts", "target"]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "README.md",
                "excerpt": "1|# RustClaw\n2|local Rust agent runtime"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer = "RustClaw is organized around a Rust core in crates, with UI, configs, and scripts around it.";

    assert!(local_compound_listing_answer_verifier_gap(&route, &journal, answer).is_none());
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
}

#[test]
fn workspace_project_summary_inventory_names_do_not_skip_model_language_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.requires_content_evidence = true;
    route.resolved_intent =
        "summarize current workspace organization from top-level directories".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-workspace-summary-inventory",
        "ask",
        "prompt",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "list_dir",
                "entries": [
                    {"name": "UI", "kind": "dir"},
                    {"name": "configs", "kind": "dir"},
                    {"name": "crates", "kind": "dir"},
                    {"name": "scripts", "kind": "dir"}
                ]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_2".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "read_range",
                "path": "README.md",
                "excerpt": "1|# RustClaw\n2|local agent runtime"
            })
            .to_string(),
        ),
        error: None,
        started_at: 2,
        finished_at: 3,
    });

    let answer = "RustClaw keeps the runtime under crates, the browser console in UI, and helper automation in scripts.";

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
    assert!(!structural_satisfaction_can_skip_verifier(
        &route, &journal, answer
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "This workspace has a clear project layout."
    ));
}

#[test]
fn structural_satisfaction_does_not_skip_missing_contract_evidence() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-structural-gap", "ask", "exists?");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "path_batch_facts",
                "facts": [{
                    "path": "/tmp/a.txt",
                    "exists": true
                }]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "/tmp/a.txt exists"
    ));
    let gap = local_missing_evidence_verifier_gap(&route, &journal).expect("missing kind evidence");
    assert_eq!(gap.missing_evidence_fields, vec!["kind"]);
    assert!(!structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "/tmp/a.txt exists"
    ));
}

#[test]
fn structural_satisfaction_skips_verifier_for_health_check_diagnostic_fields() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.requires_content_evidence = true;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-health-check-structural",
        "ask",
        "health check",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "health_check".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "clawd_process_count": 1,
                    "clawd_health_port_open": true,
                    "telegramd_process_count": 0,
                    "clawd_log": {"exists": true, "keyword_error_count": 43},
                    "telegramd_log": {"exists": true, "keyword_error_count": 1},
                    "system_health": {
                        "os_family": "linux",
                        "service_manager": "systemd",
                        "cpu_count": 8,
                        "load_avg_1m": 7.65,
                        "load_avg_5m": 6.1,
                        "load_avg_15m": 3.37,
                        "memory_available_bytes": 8403259392u64,
                        "memory_total_bytes": 15937286144u64,
                        "disk_root_available_bytes": 14794739712u64,
                        "disk_root_total_bytes": 156546629632u64,
                        "warnings": ["disk_root_low"]
                    }
                },
                "text": "{}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    let answer = concat!(
        "health_check.summary: clawd.status=running; clawd_process_count=1; ",
        "clawd_health_port_open=true; telegramd_process_count=0; ",
        "clawd_log.exists=true; clawd_log.keyword_error_count=43; ",
        "telegramd_log.exists=true; telegramd_log.keyword_error_count=1; ",
        "system_health.os_family=linux; system_health.service_manager=systemd; ",
        "system_health.cpu_count=8; system_health.load_avg_1m=7.65; ",
        "system_health.load_avg_5m=6.1; system_health.load_avg_15m=3.37; ",
        "system_health.memory_available_bytes=8403259392; ",
        "system_health.memory_total_bytes=15937286144; ",
        "system_health.disk_root_available_bytes=14794739712; ",
        "system_health.disk_root_total_bytes=156546629632; ",
        "system_health.warnings=disk_root_low."
    );

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
    assert!(structural_satisfaction_can_skip_verifier(
        &route, &journal, answer
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "health_check.summary: clawd.status=running; clawd_process_count=1."
    ));
}

#[test]
fn structural_satisfaction_skips_verifier_for_deterministic_finalizer_summary() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryEntryGroups;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-finalizer-summary-skip", "ask", "list");
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "inventory_dir",
                "names_by_kind": {
                    "dirs": ["configs"],
                    "files": ["README.md"],
                    "other": []
                },
                "counts": {"dirs": 1, "files": 1, "total": 2}
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    journal.finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    });

    assert!(structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "dirs:\n- configs\nfiles:\n- README.md"
    ));
}

#[test]
fn grounded_file_token_satisfies_file_delivery_contract_before_llm_verifier() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-answer-verifier-file-token-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("release_checklist.md");
    std::fs::write(&file, "ok").expect("write temp file");

    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-file-token", "ask", "send that file");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "path_batch_facts",
                    "facts": [{
                        "path": file.display().to_string(),
                        "fact": {
                            "kind": "file",
                            "resolved_path": file.display().to_string()
                        }
                    }]
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        &format!("FILE:{}", file.display())
    ));

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn grounded_file_token_uses_path_token_from_write_text_output() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-answer-verifier-write-text-token-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("contract_matrix_generic_delivery.txt");
    std::fs::write(&file, "generic delivery case").expect("write temp file");

    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-file-token", "ask", "send that file");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(format!("written 21 bytes to {}", file.display())),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        &format!("FILE:{}", file.display())
    ));

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn confirmed_missing_file_delivery_skips_model_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.requires_content_evidence = true;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-missing-delivery",
        "ask",
        "send definitely_missing_named_file_golden_001.txt",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "extra": {
                        "action": "find_name",
                        "count": 0,
                        "exact": false,
                        "patterns": ["definitely_missing_named_file_golden_001.txt"],
                        "results": [],
                        "root": ""
                    },
                    "text": "{\"action\":\"find_name\",\"count\":0,\"exact\":false,\"patterns\":[\"definitely_missing_named_file_golden_001.txt\"],\"results\":[],\"root\":\"\"}"
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "definitely_missing_named_file_golden_001.txt"
    ));
    assert!(!structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "FILE:/tmp/definitely_missing_named_file_golden_001.txt"
    ));
}

#[test]
fn matrix_delivery_artifact_shape_rejects_raw_command_summary_answer() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-delivery-shape", "ask", "send file");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some("done".to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, "done"
    ));
}

#[test]
fn matrix_delivery_artifact_shape_accepts_grounded_plain_path() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-answer-verifier-plain-delivery-path-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("report.md");
    std::fs::write(&file, "ok").expect("write temp file");

    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-delivery-path", "ask", "send file");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "path": file.display().to_string(),
                    "resolved_path": file.display().to_string()
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        &file.display().to_string()
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        &format!("File: {}", file.display())
    ));

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn scalar_answer_grounded_in_plain_observation_skips_llm_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-scalar", "ask", "where am I");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some("/home/guagua/rustclaw\n".to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "/home/guagua/rustclaw"
    ));
}

#[test]
fn scalar_answer_grounded_in_json_observation_skips_llm_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-json-scalar", "ask", "count them");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(json!({"count": 3, "items": ["a", "b", "c"]}).to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "3"
    ));
}

#[test]
fn quantity_comparison_size_answer_grounded_in_total_size_bytes_skips_llm_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_with_chat_finalizer());
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-size", "ask", "target size?");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "count_inventory",
                    "counts": {
                        "dirs": 7761,
                        "files": 121355,
                        "total": 129116,
                        "total_size_bytes": 57264444014_u64
                    },
                    "path": "/home/guagua/rustclaw/target"
                })
                .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        "target 目录大小约 53.3 GB，包含 129116 个项目。"
    ));
}

#[path = "answer_verifier_tests/matrix_shape_contracts.rs"]
mod matrix_shape_contracts;

#[path = "answer_verifier_tests/matrix_grounding.rs"]
mod matrix_grounding;

#[path = "answer_verifier_tests/service_control_capability_grounding.rs"]
mod service_control_capability_grounding;

#[path = "answer_verifier_tests/scalar_capability_shape.rs"]
mod scalar_capability_shape;

#[path = "answer_verifier_tests/text_protocol_boundary.rs"]
mod text_protocol_boundary;

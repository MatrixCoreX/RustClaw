use std::collections::BTreeSet;

use serde_json::json;

use super::super::{
    answer_verifier_user_request_for_prompt, execution_evidence_prompt_block,
    output_contract_prompt_block, AnswerVerifierOut,
};
use super::*;

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
    const SCHEMA_RAW: &str =
        include_str!("../../../../prompts/schemas/answer_verifier.schema.json");
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
        include_str!("../../../../prompts/layers/overlays/answer_verifier_prompt.md");
    assert!(PROMPT_RAW.contains("preserve the already required deliverable"));
    assert!(PROMPT_RAW.contains("combined final answer"));
    assert!(PROMPT_RAW.contains("include the observed listed items and the synthesis"));
}

#[test]
fn answer_verifier_output_contract_exposes_evidence_profile() {
    let mut route = route_with_mode();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.requires_content_evidence = true;

    let block = output_contract_prompt_block(&route);
    let output_contract: serde_json::Value =
        serde_json::from_str(&block).expect("output contract prompt block should be JSON");

    assert!(block.contains("\"evidence_policy\""));
    assert!(block.contains("\"compact_line\""));
    assert!(block.contains("\"evidence_profile\""));
    assert!(block.contains("\"workspace_user_docs_first\""));
    assert!(!block.contains("\"contract_marker\""));
    assert!(output_contract.get("contract_marker").is_none());
    assert_eq!(
        output_contract
            .get("final_answer_shape")
            .and_then(serde_json::Value::as_str),
        Some("project_summary_grounded_in_files")
    );
    assert_eq!(
        output_contract
            .get("final_answer_shape_class")
            .and_then(serde_json::Value::as_str),
        Some("grounded_summary")
    );
    assert!(output_contract.get("semantic_kind").is_none());
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
                "token": "sk-test-secret-token-that-should-not-leak",
                "job_id": "provider:image_generate:minimax:dry_run",
                "result_ref": "provider:image_generate:minimax:dry_run"
            })
            .to_string(),
        ),
        error: Some("password=secret-value-that-should-not-leak".to_string()),
        started_at: 1,
        finished_at: 2,
    });

    let block = execution_evidence_prompt_block(&journal);

    assert!(block.contains("\"observed_evidence\""));
    assert!(block.contains("\"structured_output_projection\""));
    assert!(block.contains("\"output_excerpt_hash\""));
    assert!(block.contains("\"error_excerpt_hash\""));
    assert!(!block.contains("\"output_excerpt\""));
    assert!(!block.contains("\"error_excerpt\""));
    assert!(!block.contains("sk-test-secret-token-that-should-not-leak"));
    assert!(!block.contains("password=secret-value-that-should-not-leak"));
    assert!(block.contains("provider:image_generate:minimax:dry_run"));
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
    assert!(block.contains(r#""command_output""#), "block: {block}");
    assert!(block.contains(r#""field": "exit_code""#), "block: {block}");
    assert!(
        !block.contains("definitely_missing_command_rustclaw_english_67890"),
        "block: {block}"
    );
}

#[test]
fn execution_failed_step_answer_uses_failed_machine_tokens_not_success_stdout() {
    let mut route = route_with_mode();
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

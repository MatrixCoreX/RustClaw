use std::collections::BTreeSet;

use serde_json::json;

use super::{
    execution_evidence_prompt_block, local_missing_evidence_verifier_gap,
    observed_scalar_values_from_evidence_map, observed_scalar_values_from_evidence_map_for_route,
    observed_single_path_values_from_evidence_map, observed_strict_list_items_from_evidence_map,
    observed_strict_list_items_from_evidence_map_for_route, observed_table_cells_from_evidence_map,
    should_verify_answer, structural_satisfaction_can_skip_verifier,
    structurally_satisfies_answer_contract, AnswerVerifierOut,
};

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
fn clarify_final_status_skips_answer_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
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
    let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
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

#[test]
fn matrix_scalar_shape_requires_plain_scalar_answer() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-matrix-scalar", "ask", "count them");
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
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "The count is 3."
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, "count: 3"
    ));
}

#[test]
fn matrix_scalar_count_shape_allows_observed_component_breakdown() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-component-count", "ask", "count dirs");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "count_inventory",
                    "counts": {
                        "total": 66,
                        "files": 40,
                        "dirs": 26
                    }
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
        "文件：40 个\n文件夹：26 个"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "总数：66 个"
    ));
}

#[test]
fn matrix_single_path_shape_accepts_root_prefixed_results() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-root-prefixed-path", "ask", "find it");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_search".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "find_name",
                    "count": 1,
                    "root": "plan",
                    "results": ["plan/agent_intelligence_architecture_plan_20260511.md"]
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
        "plan/agent_intelligence_architecture_plan_20260511.md"
    ));
}

#[test]
fn structured_keys_answer_covering_all_keys_skips_llm_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::StructuredKeys;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-keys", "ask", "list keys");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "config_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "structured_keys",
                    "exists": true,
                    "container_type": "object",
                    "count": 3,
                    "keys": ["app", "features", "paths"]
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
        "app, features, paths"
    ));
    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "app\nfeatures\npaths"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "app, features"
    ));
}

#[test]
fn matrix_strict_list_shape_rejects_unobserved_items() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-matrix-list", "ask", "list files");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "inventory_dir",
                    "names_only": true,
                    "names": ["README.md", "Cargo.toml"],
                    "entries": [
                        {"name": "README.md", "kind": "file", "path": "/tmp/repo/README.md"},
                        {"name": "Cargo.toml", "kind": "file", "path": "/tmp/repo/Cargo.toml"}
                    ]
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
        "- README.md\n- Cargo.toml"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "- README.md\n- missing.txt"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "The files are README.md and Cargo.toml."
    ));
}

#[test]
fn matrix_table_shape_requires_markdown_table_answer() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;
    route.output_contract.locator_hint = "data/app.sqlite".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-matrix-table", "ask", "list tables");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "db_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "columns": ["name"],
                    "rows": [
                        {"name": "orders"},
                        {"name": "users"}
                    ]
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
        "| name |\n| --- |\n| orders |\n| users |"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "orders, users"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "| name |\n| --- |\n| orders |"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "| name |\n| --- |\n| orders |\n| payments |"
    ));
}

#[test]
fn matrix_single_path_shape_requires_plain_grounded_path() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchivePack;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-matrix-path", "ask", "pack logs");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "archive_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "pack",
                    "archive_path": "/tmp/rustclaw/report.zip",
                    "source_paths": ["/tmp/rustclaw/report.md"]
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
        "/tmp/rustclaw/report.zip"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "Archive: /tmp/rustclaw/report.zip"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "The archive is /tmp/rustclaw/report.zip"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "/tmp/rustclaw/missing.zip"
    ));
}

#[test]
fn matrix_scalar_shape_uses_observed_evidence_map_values() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-scalar-evidence",
        "ask",
        "count them",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({"count": 3, "items": ["a", "b", "c"]}).to_string(),
        ));

    assert!(observed_scalar_values_from_evidence_map(&journal).contains("3"));
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "3"
    ));
}

#[test]
fn matrix_scalar_shape_rejects_unregistered_fallback_extractor_values() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-scalar-fallback-extractor",
        "ask",
        "count them",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "unregistered_external_skill",
            json!({"count": 3, "items": ["a", "b", "c"]}).to_string(),
        ));

    assert!(observed_scalar_values_from_evidence_map(&journal).contains("3"));
    assert!(!observed_scalar_values_from_evidence_map_for_route(&route, &journal).contains("3"));
    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, "3"
    ));
}

#[test]
fn git_repository_state_schema_answer_is_structurally_grounded() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GitRepositoryState;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-git-repository-state",
        "ask",
        "show status",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "git_basic",
            "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
        ));

    let answer = "\
git.branch=main
git.worktree=dirty
git.changed.count=2
git.changed[0]=M Cargo.toml
git.changed[1]=?? tmp/generated.txt";
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "git.branch=main\ngit.worktree=dirty\ngit.changed.count=2"
    ));
}

#[test]
fn git_repository_state_one_sentence_accepts_stable_state_fields() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GitRepositoryState;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-git-repository-state-one-sentence",
        "ask",
        "check dirty state",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "git_basic",
            "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "git.branch=main git.worktree=dirty"
    ));
}

#[test]
fn matrix_scalar_shape_accepts_admitted_external_extra_count() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-external-admitted",
        "ask",
        "count them",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "external_counter",
            json!({
                "action": "count",
                "text": "3",
                "extra": {
                    "action": "count",
                    "count": 3,
                    "results": ["a", "b", "c"]
                },
                "_matrix_admission": {
                    "schema_version": 1,
                    "source": "skills_registry",
                    "skill": "external_counter",
                    "eligible": true,
                    "extractor_kind": "structured_json",
                    "required_extra_fields": ["extra.count"]
                }
            })
            .to_string(),
        ));

    assert!(observed_scalar_values_from_evidence_map_for_route(&route, &journal).contains("3"));
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "3"
    ));
}

#[test]
fn matrix_scalar_shape_does_not_use_content_excerpt_as_field_value() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-scalar-content-excerpt",
        "ask",
        "service status",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_read",
            "fs_basic",
            json!({
                "action": "read_text_range",
                "path": "/tmp/status-notes.md",
                "excerpt": "1|running"
            })
            .to_string(),
        ));

    assert!(!observed_scalar_values_from_evidence_map(&journal).contains("1|running"));
    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, "running"
    ));
}

#[test]
fn matrix_scalar_shape_ignores_read_text_structured_fields() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    route.output_contract.requires_content_evidence = false;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-scalar-read-fields",
        "ask",
        "service status",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_read",
            "fs_basic",
            json!({
                "action": "read_text_range",
                "path": "/tmp/status-notes.md",
                "status": "running"
            })
            .to_string(),
        ));

    assert!(observed_scalar_values_from_evidence_map(&journal).contains("running"));
    assert!(
        !observed_scalar_values_from_evidence_map_for_route(&route, &journal).contains("running")
    );
    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, "running"
    ));
}

#[test]
fn matrix_strict_list_shape_ignores_read_text_list_fields() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = false;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-list-read-fields",
        "ask",
        "list files",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_read",
            "fs_basic",
            json!({
                "action": "read_text_range",
                "path": "/tmp/listing-notes.md",
                "names": ["README.md", "Cargo.toml"]
            })
            .to_string(),
        ));

    assert!(observed_strict_list_items_from_evidence_map(&journal).contains("readme.md"));
    assert!(
        !observed_strict_list_items_from_evidence_map_for_route(&route, &journal)
            .contains("readme.md")
    );
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "- README.md\n- Cargo.toml"
    ));
}

#[test]
fn matrix_strict_list_shape_uses_observed_evidence_map_values() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-list-evidence",
        "ask",
        "list files",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "action": "inventory_dir",
                "names": ["README.md", "Cargo.toml"]
            })
            .to_string(),
        ));

    let items = observed_strict_list_items_from_evidence_map(&journal);
    assert!(items.contains("readme.md"));
    assert!(items.contains("cargo.toml"));
    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "- README.md\n- Cargo.toml"
    ));
}

#[test]
fn matrix_scalar_shape_accepts_count_from_array_evidence_for_non_scalar_route_shape() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-array-count", "ask", "count rows");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "db_basic",
            json!({"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}).to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "2"
    ));
}

#[test]
fn matrix_file_path_list_shape_allows_grounded_filtered_subset() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-path-subset", "ask", "find path");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "action": "find_name",
                "results": ["plan/a.md", "plan/b.md", "docs/c.md"]
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "plan/b.md"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "plan/missing.md"
    ));
}

#[test]
fn matrix_shape_grounding_ignores_synthesis_and_verifier_steps() {
    let mut list_route = route_with_mode(crate::AskMode::planner_execute_plain());
    list_route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    list_route.output_contract.requires_content_evidence = true;
    list_route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    let mut list_journal =
        crate::task_journal::TaskJournal::for_task("task-synth-list", "ask", "list files");
    list_journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_synth",
            "synthesize_answer",
            json!({"names": ["README.md", "Cargo.toml"]}).to_string(),
        ));
    list_journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_verifier",
            "answer_verifier",
            json!({"observed_evidence": {"items": [{"kind": "filename", "excerpt": "README.md"}]}})
                .to_string(),
        ));
    assert!(!structurally_satisfies_answer_contract(
        &list_route,
        &list_journal,
        "- README.md\n- Cargo.toml"
    ));

    let mut table_route = route_with_mode(crate::AskMode::planner_execute_plain());
    table_route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    table_route.output_contract.requires_content_evidence = true;
    table_route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;
    let mut table_journal =
        crate::task_journal::TaskJournal::for_task("task-synth-table", "ask", "list tables");
    table_journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_respond",
            "respond",
            json!({"rows": [{"name": "orders"}, {"name": "users"}]}).to_string(),
        ));
    assert!(!structurally_satisfies_answer_contract(
        &table_route,
        &table_journal,
        "| name |\n| --- |\n| orders |\n| users |"
    ));

    let mut path_route = route_with_mode(crate::AskMode::planner_execute_plain());
    path_route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    path_route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchivePack;
    let mut path_journal =
        crate::task_journal::TaskJournal::for_task("task-synth-path", "ask", "pack logs");
    path_journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_think",
            "think",
            json!({"archive_path": "/tmp/rustclaw/report.zip"}).to_string(),
        ));
    assert!(!structurally_satisfies_answer_contract(
        &path_route,
        &path_journal,
        "/tmp/rustclaw/report.zip"
    ));

    let mut scalar_route = route_with_mode(crate::AskMode::planner_execute_plain());
    scalar_route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    scalar_route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    let mut scalar_journal =
        crate::task_journal::TaskJournal::for_task("task-synth-scalar", "ask", "count files");
    scalar_journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_synth",
            "synthesize_answer",
            json!({"count": 3}).to_string(),
        ));
    assert!(!structurally_satisfies_answer_contract(
        &scalar_route,
        &scalar_journal,
        "3"
    ));
}

#[test]
fn matrix_table_shape_uses_observed_evidence_map_cells() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-table-evidence",
        "ask",
        "list tables",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "db_basic",
            json!({
                "columns": ["name"],
                "rows": [
                    {"name": "orders"},
                    {"name": "users"}
                ]
            })
            .to_string(),
        ));

    let cells = observed_table_cells_from_evidence_map(&journal);
    assert!(cells.contains("orders"));
    assert!(cells.contains("users"));
    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "| name |\n| --- |\n| orders |\n| users |"
    ));
}

#[test]
fn matrix_table_shape_uses_run_cmd_results_as_table_cells() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::SqliteTableListing;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-table-run-cmd",
        "ask",
        "list tables",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "run_cmd",
            "orders\nservice_logs\nusers\n",
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "| name |\n| --- |\n| orders |\n| service_logs |\n| users |"
    ));
}

#[test]
fn matrix_single_path_shape_uses_observed_evidence_map_paths() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchivePack;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-matrix-path-evidence", "ask", "pack logs");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "archive_basic",
            json!({
                "archive_path": "/tmp/rustclaw/report.zip",
                "source_paths": ["/tmp/rustclaw/report.md"]
            })
            .to_string(),
        ));

    assert!(observed_single_path_values_from_evidence_map(&journal)
        .contains("/tmp/rustclaw/report.zip"));
    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "/tmp/rustclaw/report.zip"
    ));
}

#[test]
fn archive_unpack_summary_is_satisfied_by_observed_destination_path() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveUnpack;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-archive-unpack-summary",
        "ask",
        "unpack archive",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "archive_basic",
            "dest_path=/tmp/rustclaw-workspace/tmp/contract_matrix_unpacked\nexit=0\nArchive: /tmp/test_bundle.zip\n inflating: /tmp/rustclaw-workspace/tmp/contract_matrix_unpacked/notes.txt\n",
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "已解压到 /tmp/rustclaw-workspace/tmp/contract_matrix_unpacked，包含 notes.txt。"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "已完成解压。"
    ));
}

#[test]
fn structured_keys_answer_accepts_array_identity_values() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::StructuredKeys;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-array-keys", "ask", "list names");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "config_basic",
            json!({
                "action": "structured_keys",
                "exists": true,
                "container_type": "array",
                "count": 2,
                "identity_values": ["fs_basic", "config-basic"]
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "`fs_basic`, `config-basic`"
    ));
}

#[test]
fn structured_keys_answer_uses_observed_action_when_semantic_label_missing() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-keys-missing-label", "ask", "keys");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "config_basic",
            json!({
                "action": "structured_keys",
                "exists": true,
                "container_type": "object",
                "count": 3,
                "keys": ["app", "features", "paths"]
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "app, features, paths"
    ));
}

#[test]
fn markdown_heading_answer_grounded_in_read_range_skips_llm_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-read-heading", "ask", "read it");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "action": "read_range",
                "excerpt": "1|# RustClaw\n2|\n3|<img src=\"./RustClaw.png\" width=\"420\" />\n4|",
                "path": "README.md"
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "RustClaw"
    ));
    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "# RustClaw"
    ));
}

#[test]
fn existence_with_path_answer_grounded_by_existing_path_fact_skips_llm_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-exists", "ask", "check path");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "system_basic",
            json!({
                "action": "path_batch_facts",
                "facts": [{
                    "exists": true,
                    "path": "README.md",
                    "fact": {
                        "kind": "file",
                        "resolved_path": "/repo/README.md"
                    }
                }]
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "有，路径：/repo/README.md"
    ));
}

#[test]
fn directory_purpose_summary_uses_largest_path_batch_fact_for_structural_skip() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose", "ask", "summarize dir");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "action": "path_batch_facts",
                "facts": [
                    {
                        "exists": true,
                        "path": "/repo/prompts/schemas/contract_repair_judge.schema.json",
                        "fact": {
                            "kind": "file",
                            "path": "prompts/schemas/contract_repair_judge.schema.json",
                            "size_bytes": 6112
                        }
                    },
                    {
                        "exists": true,
                        "path": "/repo/prompts/schemas/intent_normalizer.schema.json",
                        "fact": {
                            "kind": "file",
                            "path": "prompts/schemas/intent_normalizer.schema.json",
                            "size_bytes": 13124
                        }
                    }
                ]
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "synthesize_answer",
            "最大的是 contract_repair_judge.schema.json（6112 字节）。".to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "最大的是 intent_normalizer.schema.json（13124 字节），它描述意图归一化对象。"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "最大的是 contract_repair_judge.schema.json（6112 字节）。"
    ));
}

#[test]
fn existence_with_path_answer_grounded_by_missing_path_fact_skips_llm_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-missing", "ask", "check path");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "system_basic",
            json!({
                "action": "path_batch_facts",
                "facts": [{
                    "exists": false,
                    "path": "missing.txt",
                    "error": "not found"
                }]
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "未找到 `missing.txt`，请确认路径后再继续。"
    ));
}

#[test]
fn existence_with_path_answer_ignores_doc_parse_path_facts() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.requires_content_evidence = false;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-exists-doc-parse", "ask", "check path");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_parse",
            "doc_parse",
            json!({
                "action": "parse_doc",
                "path": "README.md",
                "facts": [{
                    "exists": true,
                    "path": "README.md",
                    "fact": {
                        "kind": "file",
                        "resolved_path": "/repo/README.md"
                    }
                }]
            })
            .to_string(),
        ));

    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "有，路径：/repo/README.md"
    ));
}

#[test]
fn existence_with_path_answer_ignores_read_text_path_facts() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    route.output_contract.requires_content_evidence = false;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-exists-read-text", "ask", "check path");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_read",
            "fs_basic",
            json!({
                "action": "read_text_range",
                "path": "README.md",
                "facts": [{
                    "exists": true,
                    "path": "README.md",
                    "fact": {
                        "kind": "file",
                        "resolved_path": "/repo/README.md"
                    }
                }]
            })
            .to_string(),
        ));

    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "有，路径：/repo/README.md"
    ));
}

#[test]
fn exact_single_run_cmd_output_skips_llm_verifier_without_scalar_contract() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let mut journal = crate::task_journal::TaskJournal::for_task("task-run-cmd", "ask", "run it");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "run_cmd".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some("line 1\nline 2\n".to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some("line 1\nline 2".to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "line 1\nline 2"
    ));
}

#[test]
fn exact_repeated_run_cmd_output_skips_llm_verifier() {
    let route = route_with_mode(crate::AskMode::planner_execute_plain());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-retry-command", "ask", "run it");
    for idx in 1..=3 {
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: format!("step_{idx}"),
                skill: "run_cmd".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("line 1\nline 2\n".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });
    }

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "line 1\nline 2"
    ));
}

#[test]
fn exact_run_cmd_output_skip_rejects_mixed_external_outputs() {
    let route = route_with_mode(crate::AskMode::planner_execute_plain());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-two-commands", "ask", "run both");
    for (idx, output) in ["first", "second"].into_iter().enumerate() {
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: format!("step_{}", idx + 1),
                skill: "run_cmd".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(output.to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });
    }

    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, "second"
    ));
}

#[test]
fn free_shape_non_command_plain_observation_still_uses_llm_verifier() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-free", "ask", "summarize output");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some("ok".to_string()),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });

    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, "ok"
    ));
}

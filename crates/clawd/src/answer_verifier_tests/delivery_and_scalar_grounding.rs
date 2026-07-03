use super::*;

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

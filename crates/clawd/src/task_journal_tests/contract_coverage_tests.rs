use super::*;

fn answer_contract_from_route(
    route: &crate::IntentOutputContract,
) -> crate::answer_verifier::AnswerContract {
    crate::answer_verifier::AnswerContract::new("test request", route.clone())
}

#[test]
fn config_validation_evidence_coverage_accepts_valid_flag() {
    let mut journal = TaskJournal::for_task("task-config-validation", "ask", "验证配置");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ConfigValidation,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "validate_structured",
                "path": "configs/config.toml",
                "format": "toml",
                "valid": true,
                "root_type": "object",
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
}

#[test]
fn config_mutation_plan_change_evidence_counts_as_valid_plan_proof() {
    let mut journal = TaskJournal::for_task("task-config-plan", "ask", "preview config change");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ConfigMutation);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "configs/config.toml".to_string();
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_edit".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "plan_config_change",
                    "path": "configs/config.toml",
                    "resolved_path": "/repo/configs/config.toml",
                    "field_path": "skills.skill_switches.example",
                    "old_value": null,
                    "new_value": true,
                    "would_change": true,
                    "requires_confirmation": true
                },
                "text": "{\"action\":\"plan_config_change\",\"path\":\"configs/config.toml\",\"field_path\":\"skills.skill_switches.example\",\"new_value\":true,\"would_change\":true,\"requires_confirmation\":true}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("valid"));
}

#[test]
fn config_mutation_apply_validated_flag_counts_as_valid_evidence() {
    let mut journal = TaskJournal::for_task("task-config-apply", "ask", "apply config change");
    let mut route = route_for_semantic(crate::OutputSemanticKind::ConfigMutation);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::Path;
    route.locator_hint = "configs/config.toml".to_string();
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_edit".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "apply_config_change",
                "path": "configs/config.toml",
                "resolved_path": "/repo/configs/config.toml",
                "field_path": "skills.skill_switches.example",
                "old_value": null,
                "new_value": true,
                "applied": true,
                "validated": true
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("path"));
    assert!(coverage.observed_canonical.contains("valid"));
}

#[test]
fn trace_json_reports_required_vs_observed_evidence_coverage() {
    let mut journal = TaskJournal::for_task("task-evidence-coverage", "ask", "列出文件名");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({"action": "list_dir", "names": ["Cargo.toml", "README.md"]}).to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let coverage = trace
        .get("evidence_coverage")
        .expect("evidence coverage should be present");
    assert_eq!(
        coverage
            .get("required_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["candidates"])
    );
    assert_eq!(
        coverage
            .get("missing_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(Vec::<&str>::new())
    );
    assert!(coverage
        .get("observed_canonical")
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("candidates"))));
    assert!(coverage
        .get("observed_extractors")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("fs_basic.list_dir.structured_json_v1"))));
    assert!(coverage
        .pointer("/observed_evidence_sources/candidates")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("fs_basic.list_dir.structured_json_v1"))));
    assert!(coverage
        .get("source_refs")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("fs_basic.list_dir.structured_json_v1"))));
    assert_eq!(
        coverage.get("confidence").and_then(Value::as_f64),
        Some(1.0)
    );
    assert_eq!(
        coverage.get("repair_eligible").and_then(Value::as_bool),
        Some(false)
    );
    let summary = journal.to_summary_json();
    assert_eq!(
        summary
            .get("task_outcome")
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str),
        Some("in_progress")
    );
}

#[test]
fn structured_selector_evidence_coverage_accepts_guard_findings() {
    let mut journal = TaskJournal::for_task("task-config-risk-evidence", "ask", "检查配置风险");
    let route = crate::IntentOutputContract {
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        requires_content_evidence: true,
        selection: crate::OutputSelectionContract {
            structured_field_selector: Some("candidates,count".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "config_edit".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "guard_config",
                "format": "toml",
                "path": "configs/config.toml",
                "resolved_path": "/home/guagua/rustclaw/configs/config.toml",
                "risk_count": 2,
                "risks": [
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

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    let trace = journal.to_trace_json();
    let items = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .and_then(|observed| observed.get("items"))
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");

    assert!(coverage.is_complete());
    assert_eq!(coverage.required_evidence, vec!["candidates", "count"]);
    assert!(coverage.observed_canonical.contains("candidates"));
    assert!(coverage.observed_canonical.contains("count"));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("risks[1]")
            && item.get("excerpt").and_then(Value::as_str)
                == Some("tools.allow_path_outside_workspace=true")
            && item.get("redacted").is_none()
    }));
}

#[test]
fn filesystem_mutation_result_accepts_kb_ingest_path_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-kb-ingest-evidence",
        "ask",
        "ingest README into demo_docs_nl",
    );
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FilesystemMutationResult,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        locator_hint: "README.md".to_string(),
        requires_content_evidence: true,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "kb".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "ingest",
                "status": "ok",
                "namespace": "demo_docs_nl",
                "path": "README.md",
                "paths": ["README.md"],
                "stats": {
                    "ingested_docs": 1,
                    "total_docs": 1,
                    "total_chunks": 3
                }
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    let trace = journal.to_trace_json();

    assert!(coverage.is_complete());
    assert_eq!(coverage.required_evidence, vec!["path"]);
    assert!(coverage.observed_canonical.contains("path"));
    assert!(trace
        .pointer("/step_results/0/observed_evidence/extractor/extractor_ref")
        .and_then(Value::as_str)
        .is_some_and(|extractor| extractor == "kb.ingest.structured_json_v1"));
}

#[test]
fn evidence_coverage_ignores_failed_and_synthesis_outputs() {
    let mut journal = TaskJournal::for_task(
        "task-evidence-coverage-filter",
        "ask",
        "summarize file content",
    );
    let route = crate::IntentOutputContract {
        requires_content_evidence: true,
        semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "README.md".to_string(),
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_failed".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: Some(json!({"content": "failed read should not count"}).to_string()),
        error: Some("read failed".to_string()),
        started_at: 1,
        finished_at: 2,
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_synthesis".to_string(),
        skill: "synthesize_answer".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({"content": "model synthesis should not count as observed content"}).to_string(),
        ),
        error: None,
        started_at: 3,
        finished_at: 4,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);

    assert!(!coverage.is_complete(), "coverage: {coverage:?}");
    assert_eq!(
        coverage.missing_evidence,
        vec!["any_of(candidates|content_excerpt|count|field_value)"]
    );
    assert!(!coverage.observed_canonical.contains("content_excerpt"));
}

#[test]
fn wrapped_machine_read_action_counts_as_text_content_read_without_skill_branch() {
    let mut journal = TaskJournal::for_task(
        "task-wrapped-read-range-content",
        "ask",
        "inspect bounded file content",
    );
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_read".to_string(),
        skill: "fixture_reader".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "logs/model_io.log",
                    "excerpt": "1|{\"call_id\":\"abc\"}"
                },
                "text": "{}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    assert!(crate::task_journal::step_reads_text_content(
        &journal.step_results[0]
    ));
}

#[test]
fn raw_command_output_error_step_supplies_command_output_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-run-cmd-failure-evidence",
        "ask",
        "cat /definitely_missing_rustclaw_contract_case",
    );
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExecutionFailedStep,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "run_cmd".to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(crate::skills::structured_skill_error_from_parts(
            "run_cmd",
            "nonzero_exit",
            "Command failed with exit code 1",
            Some("linux"),
            Some(json!({
                "command": "cat /definitely_missing_rustclaw_contract_case",
                "exit_code": 1,
                "stderr": "cat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)\n",
                "stdout": Value::Null,
            })),
        )),
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert!(coverage.observed_canonical.contains("field_value"));

    let trace = journal.to_trace_json();
    let items = trace
        .get("step_results")
        .and_then(Value::as_array)
        .and_then(|steps| steps.first())
        .and_then(|step| step.get("observed_evidence"))
        .and_then(|observed| observed.get("items"))
        .and_then(Value::as_array)
        .expect("observed evidence items should be present");
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("stderr")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|value| value.contains("No such file or directory"))
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("exit_code")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "1")
    }));
    assert!(items.iter().any(|item| {
        item.get("field").and_then(Value::as_str) == Some("error_kind")
            && item
                .get("excerpt")
                .and_then(Value::as_str)
                .is_some_and(|value| value == "nonzero_exit")
    }));
    assert!(!items
        .iter()
        .any(|item| item.get("field").and_then(Value::as_str) == Some("error_text")));
}

#[test]
fn summary_json_includes_machine_readable_task_outcome() {
    let mut journal = TaskJournal::for_task("task-outcome", "ask", "列出文件名");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.record_final_status(TaskJournalFinalStatus::Success);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"names": ["Cargo.toml", "README.md"]}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let outcome = journal
        .to_summary_json()
        .get("task_outcome")
        .cloned()
        .expect("task outcome");

    assert_eq!(
        outcome.get("state").and_then(Value::as_str),
        Some("completed")
    );
    assert_eq!(
        outcome.get("final_answer_shape").and_then(Value::as_str),
        Some("name_list")
    );
    assert_eq!(
        outcome
            .get("missing_evidence_count")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        outcome.get("message_key").and_then(Value::as_str),
        Some("clawd.task_outcome.completed")
    );
    assert_eq!(
        outcome.get("next_action_kind").and_then(Value::as_str),
        Some("review_result")
    );
    assert_eq!(
        outcome.get("render_owner").and_then(Value::as_str),
        Some("finalizer_or_ui_i18n")
    );
    assert!(outcome.get("message_zh").is_none());
    assert!(outcome.get("next_step_en").is_none());
}

#[test]
fn trace_json_reports_missing_required_evidence() {
    let mut journal = TaskJournal::for_task("task-evidence-missing", "ask", "这个路径是否存在");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"path": "/tmp/missing.txt", "exists": false}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let coverage = trace
        .get("evidence_coverage")
        .expect("evidence coverage should be present");
    assert_eq!(
        coverage
            .get("missing_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["kind"])
    );
    assert_eq!(
        coverage.get("repair_eligible").and_then(Value::as_bool),
        Some(true)
    );
    assert!(coverage
        .get("confidence")
        .and_then(Value::as_f64)
        .is_some_and(|value| value > 0.0 && value < 1.0));
}

#[test]
fn trace_json_uses_evidence_expression_for_confirmed_absence() {
    let mut journal = TaskJournal::for_task("task-evidence-absence", "ask", "这个路径是否存在");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "path": "/tmp/missing.txt",
                "exists": false,
                "kind": "missing"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert!(coverage.is_complete());
    assert!(coverage.observed_canonical.contains("exists_false"));

    let trace = journal.to_trace_json();
    let coverage = trace
        .get("evidence_coverage")
        .expect("evidence coverage should be present");
    assert_eq!(
        coverage
            .get("evidence_expression")
            .and_then(|value| value.get("negative_evidence"))
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["exists_false"])
    );
    assert_eq!(
        coverage
            .get("missing_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(Vec::<&str>::new())
    );
}

#[test]
fn trace_json_reports_missing_evidence_expression_alternative() {
    let mut journal = TaskJournal::for_task("task-evidence-missing-alt", "ask", "这个路径是否存在");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"path": "/tmp/maybe.txt", "kind": "file"}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);
    assert_eq!(
        coverage.missing_evidence,
        vec!["one_of(exists_false|exists_true)"]
    );
}

#[test]
fn non_content_route_ignores_doc_parse_observation_as_structured_evidence() {
    let mut journal = TaskJournal::for_task(
        "task-non-content-doc-parse-evidence",
        "ask",
        "service status",
    );
    let mut route = route_for_semantic(crate::OutputSemanticKind::ServiceStatus);
    route.requires_content_evidence = false;
    journal.record_output_contract(&route.clone());
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_parse",
        "doc_parse",
        json!({
            "action": "parse_doc",
            "path": "/tmp/service-notes.md",
            "status": "running",
            "content": "operator notes say the service should be running"
        })
        .to_string(),
    ));

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);

    assert!(!coverage.is_complete());
    assert_eq!(coverage.missing_evidence, vec!["field_value"]);
    assert!(!coverage.observed_canonical.contains("field_value"));
}

#[test]
fn trace_json_counts_nested_builtin_tool_evidence() {
    let mut journal = TaskJournal::for_task("task-nested-evidence", "ask", "这个路径是否存在");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
        locator_kind: crate::OutputLocatorKind::Path,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "action": "path_batch_facts",
                "facts": [{
                    "path": "/tmp/present.txt",
                    "exists": true,
                    "kind": "file"
                }]
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let coverage = trace
        .get("evidence_coverage")
        .expect("evidence coverage should be present");
    assert_eq!(
        coverage
            .get("missing_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(Vec::<&str>::new())
    );
    assert!(coverage
        .get("observed_fields")
        .and_then(Value::as_array)
        .is_some_and(|items| items
            .iter()
            .any(|item| item.as_str() == Some("facts[0].path"))));
}

#[test]
fn trace_json_includes_task_level_evidence_policy_snapshot() {
    let mut journal = TaskJournal::for_task("task-contract-snapshot", "ask", "列出文件名");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());

    let trace = journal.to_trace_json();
    let snapshot = trace
        .get("evidence_policy")
        .expect("evidence-policy snapshot should be present");

    assert_eq!(
        snapshot.get("contract_match").and_then(Value::as_str),
        Some("file_names")
    );
    assert_eq!(
        snapshot
            .get("required_evidence")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
        Some(vec!["candidates"])
    );
    assert_eq!(
        snapshot.get("final_answer_shape").and_then(Value::as_str),
        Some("name_list")
    );
    assert!(snapshot
        .get("evidence_policy_hash")
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
    let runtime_snapshot = trace
        .get("runtime_contract_snapshot")
        .expect("runtime contract snapshot should be present");
    assert_eq!(
        runtime_snapshot
            .get("contract")
            .and_then(|value| value.get("contract_match"))
            .and_then(Value::as_str),
        Some("file_names")
    );
    assert!(runtime_snapshot
        .get("compact_contract_block")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| !hash.is_empty()));
}

#[test]
fn step_trace_includes_contract_and_action_policy_for_success() {
    let mut journal = TaskJournal::for_task("task-step-contract", "ask", "列出文件名");
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.record_plan_result(&crate::PlanResult {
        goal: "list file names".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_skill".to_string(),
            skill: "fs_basic".to_string(),
            args: json!({"action": "list_dir", "path": "."}),
            depends_on: Vec::new(),
            why: String::new(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(json!({"items": [{"path": "README.md"}]}).to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let trace = journal.to_trace_json();
    let step_contract = trace
        .pointer("/step_results/0/contract")
        .expect("step contract trace should be present");

    assert_eq!(
        step_contract.get("contract_match").and_then(Value::as_str),
        Some("file_names")
    );
    assert!(step_contract.get("contract_marker").is_none());
    assert!(step_contract.get("semantic_kind").is_none());
    assert_eq!(
        step_contract
            .get("final_answer_shape")
            .and_then(Value::as_str),
        Some("name_list")
    );
    assert_eq!(
        step_contract
            .get("action_policy")
            .and_then(|value| value.get("decision"))
            .and_then(Value::as_str),
        Some("allowed")
    );
    assert_eq!(
        step_contract
            .get("action_policy")
            .and_then(|value| value.get("action_ref"))
            .and_then(Value::as_str),
        Some("fs_basic.list_dir")
    );
    assert!(trace
        .pointer("/step_results/0/observed_evidence/items")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty()));
}

#[test]
fn db_schema_version_action_evidence_overrides_stale_existence_route_contract() {
    let mut journal = TaskJournal::for_task(
        "task-db-schema-version",
        "ask",
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite",
    );
    let mut route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ExistenceWithPath,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
            .to_string(),
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.record_plan_result(&crate::PlanResult {
        goal: "read sqlite schema version".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "db_basic".to_string(),
            args: json!({
                "action": "schema_version",
                "db_path": "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "db_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "schema_version",
                    "db_path": "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite",
                    "field_value": {"schema_version": 3},
                    "schema_version": 3
                },
                "text": "{\"columns\":[\"schema_version\"],\"rows\":[{\"schema_version\":3}]}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert_eq!(coverage.required_evidence, vec!["field_value"]);
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.observed_canonical.contains("schema_version"));
    assert!(coverage.evidence_expression.is_none());

    route.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    assert!(
        crate::answer_verifier::local_missing_evidence_verifier_gap(
            &answer_contract_from_route(&route),
            &journal,
        )
        .is_none(),
        "schema_version action evidence should not be blocked by stale existence route contract"
    );
}

#[test]
fn runtime_status_action_evidence_overrides_generic_path_route_contract() {
    let mut journal = TaskJournal::for_task(
        "task-runtime-status-cwd",
        "ask",
        "current working directory",
    );
    let route = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::None,
        response_shape: crate::OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
        ..Default::default()
    };
    journal.record_output_contract(&route.clone());
    journal.record_plan_result(&crate::PlanResult {
        goal: "return cwd".to_string(),
        missing_slots: Vec::new(),
        needs_confirmation: false,
        output_contract: None,
        steps: vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "system_basic".to_string(),
            args: json!({
                "action": "runtime_status",
                "kind": "current_working_directory"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
        planner_notes: String::new(),
        plan_kind: crate::PlanKind::Single,
        raw_plan_text: String::new(),
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "system_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": {
                    "action": "runtime_status",
                    "command_output": "/home/guagua/rustclaw",
                    "field_value": "/home/guagua/rustclaw",
                    "kind": "current_working_directory",
                    "value": "/home/guagua/rustclaw"
                },
                "text": "{\"action\":\"runtime_status\",\"command_output\":\"/home/guagua/rustclaw\",\"field_value\":\"/home/guagua/rustclaw\",\"kind\":\"current_working_directory\",\"value\":\"/home/guagua/rustclaw\"}"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });

    let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);

    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    assert_eq!(coverage.required_evidence, vec!["field_value"]);
    assert!(coverage.observed_canonical.contains("field_value"));
    assert!(coverage.evidence_expression.is_none());
    assert!(
        crate::answer_verifier::local_missing_evidence_verifier_gap(
            &answer_contract_from_route(&route),
            &journal,
        )
        .is_none(),
        "runtime_status action evidence should not be blocked by generic path route contract"
    );

    assert!(crate::answer_verifier::local_missing_evidence_verifier_gap(
        &answer_contract_from_route(&route),
        &journal,
    )
    .is_none());
}

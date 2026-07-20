use super::*;

#[test]
fn single_file_delivery_rejects_token_mixed_with_prose() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-answer-verifier-token-only-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create temp root");
    let file = root.join("token_only_delivery.md");
    std::fs::write(&file, "ok").expect("write temp file");

    let mut route = route_with_mode();
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-token-only", "ask", "send file");
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
        &format!("FILE:{}", file.display())
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        &format!("FILE:{}\nready", file.display())
    ));

    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn matrix_scalar_shape_rejects_unregistered_fallback_extractor_values() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.selection.structured_field_selector = Some("count".to_string());
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
fn matrix_scalar_shape_accepts_admitted_external_extra_count() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.selection.structured_field_selector = Some("count".to_string());
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
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
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
fn scalar_json_read_range_candidate_can_satisfy_field_value_gap() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-scalar-json-read-range",
        "ask",
        "read package field",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_read",
            "fs_basic",
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "/tmp/package.json",
                    "excerpt": "1|{\n2|  \"name\": \"rustclaw\",\n3|  \"private\": true\n4|}"
                },
                "text": "{\"action\":\"read_range\",\"path\":\"/tmp/package.json\",\"excerpt\":\"1|{\\n2|  \\\"name\\\": \\\"rustclaw\\\",\\n3|  \\\"private\\\": true\\n4|}\"}"
            })
            .to_string(),
        ));

    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "rustclaw"
    ));
    assert!(structural_satisfaction_can_skip_verifier(
        &route, &journal, "rustclaw"
    ));
}

#[test]
fn exact_observation_output_bounded_read_excerpt_can_skip_verifier() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/clawd-dev.log".to_string();
    route.output_contract.configure_exact_command_output();
    let observed_line = "2026-05-15T15:58:11Z WARN provider auth failed code=401";
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-raw-read-tail", "ask", "tail log");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_read",
            "fs_basic",
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "/tmp/clawd-dev.log",
                    "resolved_path": "/tmp/clawd-dev.log",
                    "mode": "tail",
                    "requested_n": 1,
                    "excerpt": format!("99|{observed_line}")
                },
                "text": json!({
                    "action": "read_range",
                    "path": "/tmp/clawd-dev.log",
                    "resolved_path": "/tmp/clawd-dev.log",
                    "mode": "tail",
                    "requested_n": 1,
                    "excerpt": format!("99|{observed_line}")
                })
                .to_string()
            })
            .to_string(),
        ));

    assert!(local_missing_evidence_verifier_gap(&route, &journal).is_none());
    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        observed_line
    ));
    assert!(structural_satisfaction_can_skip_verifier(
        &route,
        &journal,
        observed_line
    ));
}

#[test]
fn exact_observation_output_bounded_read_excerpt_respects_locator_path() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/expected.log".to_string();
    route.output_contract.configure_exact_command_output();
    let observed_line = "line from another file";
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-raw-read-wrong-path", "ask", "tail log");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_read",
            "fs_basic",
            json!({
                "action": "read_text_range",
                "path": "/tmp/other.log",
                "resolved_path": "/tmp/other.log",
                "mode": "tail",
                "n": 1,
                "excerpt": format!("7|{observed_line}")
            })
            .to_string(),
        ));

    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        observed_line
    ));
}

#[test]
fn matrix_scalar_shape_ignores_read_text_structured_fields() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
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
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
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
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.selection.list_selector.target_kind =
        crate::OutputScalarCountTargetKind::File;
    route
        .output_contract
        .selection
        .list_selector
        .target_kind_specified = true;
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
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.selection.structured_field_selector = Some("count".to_string());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-array-count", "ask", "count rows");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "db_basic",
            json!({"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}).to_string(),
        ));

    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, "2"
    ));
}

#[test]
fn matrix_file_path_list_shape_allows_grounded_filtered_subset() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.selection.structured_field_selector = Some("path".to_string());
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
fn file_paths_contract_path_list_is_grounded() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.selection.structured_field_selector = Some("path".to_string());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-fs-path-capability-ref", "ask", "find");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "action": "find_entries",
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
    let mut list_route = route_with_mode();
    list_route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    list_route.output_contract.requires_content_evidence = true;
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

    let mut path_route = route_with_mode();
    path_route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    path_route
        .output_contract
        .selection
        .structured_field_selector = Some("path".to_string());
    let mut path_journal =
        crate::task_journal::TaskJournal::for_task("task-synth-path", "ask", "write report");
    path_journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_think",
            "think",
            json!({"path": "/tmp/rustclaw/report.md"}).to_string(),
        ));
    assert!(!structurally_satisfies_answer_contract(
        &path_route,
        &path_journal,
        "/tmp/rustclaw/report.md"
    ));

    let mut scalar_route = route_with_mode();
    scalar_route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    scalar_route
        .output_contract
        .selection
        .structured_field_selector = Some("count".to_string());
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
fn matrix_single_path_shape_uses_observed_evidence_map_paths() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.selection.structured_field_selector = Some("path".to_string());
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-matrix-path-evidence",
        "ask",
        "write report",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "action": "write_text",
                "path": "/tmp/rustclaw/report.md"
            })
            .to_string(),
        ));

    assert!(
        observed_single_path_values_from_evidence_map(&journal).contains("/tmp/rustclaw/report.md")
    );
    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "/tmp/rustclaw/report.md"
    ));
}

#[test]
fn path_inspection_answer_still_uses_model_verifier_after_existing_path_fact() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.selection.structured_field_selector = Some("exists,path".to_string());
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

    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "有，路径：/repo/README.md"
    ));
}

#[test]
fn path_inspection_answer_still_uses_model_verifier_after_missing_path_fact() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.selection.structured_field_selector = Some("exists,path".to_string());
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

    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "未找到 `missing.txt`，请确认路径后再继续。"
    ));
}

#[test]
fn path_inspection_answer_ignores_doc_parse_path_facts() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.selection.structured_field_selector = Some("exists,path".to_string());
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
fn path_inspection_answer_ignores_read_text_path_facts() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.selection.structured_field_selector = Some("exists,path".to_string());
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
    let mut route = route_with_mode();
    route.output_contract.configure_exact_command_output();
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
    let mut route = route_with_mode();
    route.output_contract.configure_exact_command_output();
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
    let mut route = route_with_mode();
    route.output_contract.configure_exact_command_output();
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
    let mut route = route_with_mode();
    route.output_contract.configure_exact_command_output();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
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

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
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFileDelivery;
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
fn recent_artifacts_judgment_skips_verifier_when_content_paths_are_grounded() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.requires_content_evidence = true;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-recent-artifacts-grounded",
        "ask",
        "classify recent log artifacts",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "extra": {
                    "action": "inventory_dir",
                    "counts": {"dirs": 0, "files": 2, "hidden": 0, "total": 2},
                    "entries": [
                        {
                            "kind": "file",
                            "modified_ts": 1781150839,
                            "name": "clawd.run.log",
                            "path": "logs/clawd.run.log",
                            "size_bytes": 25556169
                        },
                        {
                            "kind": "file",
                            "modified_ts": 1781150839,
                            "name": "model_io.log",
                            "path": "logs/model_io.log",
                            "size_bytes": 239412652
                        }
                    ],
                    "names": ["clawd.run.log", "model_io.log"],
                    "names_by_kind": {
                        "dirs": [],
                        "files": ["clawd.run.log", "model_io.log"],
                        "other": []
                    },
                    "path": "/repo/logs",
                    "resolved_path": "/repo/logs",
                    "sort_by": "mtime_desc"
                },
                "text": "{\"action\":\"inventory_dir\"}"
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "logs/clawd.run.log",
                    "excerpt": "1|2026-06-10T23:36:51Z INFO startup config_path=/repo/configs/config.toml"
                },
                "text": "{\"action\":\"read_range\"}"
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "fs_basic",
            json!({
                "extra": {
                    "action": "read_text_range",
                    "path": "logs/model_io.log",
                    "excerpt": "1|{\"call_id\":\"abc\",\"clean_response_preview\":\"{}\"}"
                },
                "text": "{\"action\":\"read_text_range\"}"
            })
            .to_string(),
        ));

    let answer =
        "clawd.run.log and model_io.log are the two recent artifacts and both are runtime logs.";

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, answer
    ));
    assert!(structural_satisfaction_can_skip_verifier(
        &route, &journal, answer
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "clawd.run.log is a runtime log."
    ));
}

#[test]
fn recent_artifacts_judgment_uses_truncated_read_range_path_tokens() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.requires_content_evidence = true;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-recent-artifacts-truncated",
        "ask",
        "classify recent log artifacts",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "extra": {
                    "action": "inventory_dir",
                    "counts": {"dirs": 0, "files": 2, "hidden": 0, "total": 2},
                    "entries": [
                        {"kind": "file", "name": "clawd.run.log", "path": "logs/clawd.run.log", "size_bytes": 25556169},
                        {"kind": "file", "name": "model_io.log", "path": "logs/model_io.log", "size_bytes": 239412652}
                    ],
                    "names": ["clawd.run.log", "model_io.log"],
                    "sort_by": "mtime_desc"
                },
                "text": "{\"action\":\"inventory_dir\"}"
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            "{\"extra\":{\"action\":\"read_range\",\"path\":\"logs/clawd.run.log\",\"excerpt\":\"1|startup config_path=/repo/configs/config.toml...(truncated)",
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_3",
            "fs_basic",
            "{\"extra\":{\"action\":\"read_text_range\",\"path\":\"logs/model_io.log\",\"excerpt\":\"1|{\\\"call_id\\\":\\\"abc\\\"...(truncated)",
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "clawd.run.log and model_io.log look like runtime logs.",
    ));
}

#[test]
fn matrix_scalar_shape_rejects_unregistered_fallback_extractor_values() {
    let mut route = route_with_mode();
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
fn matrix_scalar_shape_accepts_admitted_external_extra_count() {
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
fn scalar_json_read_range_candidate_can_satisfy_field_value_gap() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
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
fn raw_command_output_bounded_read_excerpt_can_skip_verifier() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/clawd-dev.log".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
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
fn raw_command_output_bounded_read_excerpt_respects_locator_path() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/expected.log".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
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
fn service_status_port_answer_uses_complete_successful_socket_observation() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-service-ports",
        "ask",
        "inspect listening ports",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_ports",
            "process_basic",
            "exit=0\nState  Recv-Q Send-Q Local Address:Port  Peer Address:PortProcess\nLISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:*\nLISTEN 0 4096 0.0.0.0:8787 0.0.0.0:* users:((\"clawd\",pid=1,fd=3))\nLISTEN 0 4096 0.0.0.0:22 0.0.0.0:*\nLISTEN 0 4096 0.0.0.0:80 0.0.0.0:*\nLISTEN 0 4096 127.0.0.1:7897 0.0.0.0:*\nLISTEN 0 4096 127.0.0.54:53 0.0.0.0:*\nLISTEN 0 4096 127.0.0.1:33331 0.0.0.0:* users:((\"clash-verge\",pid=2,fd=4))\nLISTEN 0 4096 127.0.0.1:631 0.0.0.0:*\nLISTEN 0 4096 [::]:22 [::]:*\nLISTEN 0 4096 [::]:80 [::]:*\nLISTEN 0 4096 [::1]:631 [::]:*",
        ));
    let candidate = "\
| Port | Bind | Note |
| --- | --- | --- |
| 22 | 0.0.0.0:22 / [::]:22 | ssh |
| 80 | 0.0.0.0:80 / [::]:80 | web |
| 8787 | 0.0.0.0:8787 | clawd |
| 53 | 127.0.0.53:53 / 127.0.0.54:53 | local dns |
| 631 | 127.0.0.1:631 / [::1]:631 | local print |
| 7897 | 127.0.0.1:7897 | local proxy |
| 33331 | 127.0.0.1:33331 | local app |";

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, candidate
    ));
}

#[test]
fn service_status_contract_port_answer_is_grounded() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-service-ports-capability-ref",
        "ask",
        "inspect listening ports",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_ports",
            "process_basic",
            "port.count=2\nport[0].number=22\nport[0].local=0.0.0.0:22\nport[1].number=80\nport[1].local=0.0.0.0:80",
        ));
    let candidate = "\
| Port | Bind |
| --- | --- |
| 22 | 0.0.0.0:22 |
| 80 | 0.0.0.0:80 |";

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, candidate
    ));
}

#[test]
fn service_status_port_answer_rejects_unobserved_candidate_port() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-service-ports-unobserved",
        "ask",
        "inspect listening ports",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_ports",
            "process_basic",
            "port.count=2\nport[0].number=22\nport[0].local=0.0.0.0:22\nport[1].number=80\nport[1].local=0.0.0.0:80",
        ));
    let candidate = "\
| Port | Bind |
| --- | --- |
| 22 | 0.0.0.0:22 |
| 80 | 0.0.0.0:80 |
| 443 | 0.0.0.0:443 |";

    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, candidate
    ));
}

#[test]
fn matrix_strict_list_shape_ignores_read_text_list_fields() {
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
fn file_paths_contract_path_list_is_grounded() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
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

    let mut path_route = route_with_mode();
    path_route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    path_route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFilePathReport;
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
fn matrix_single_path_shape_uses_observed_evidence_map_paths() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::GeneratedFilePathReport;
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
fn structured_keys_answer_accepts_array_identity_values() {
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
fn markdown_heading_answer_grounded_in_wrapped_read_range_skips_llm_verifier() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-wrapped-read-heading", "ask", "read it");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "extra": {
                    "action": "read_range",
                    "excerpt": "1|# Service Notes\n2|\n3|fixture body",
                    "path": "service_notes.md"
                },
                "text": "{\"action\":\"read_range\",\"excerpt\":\"1|# Service Notes\\n2|\\n3|fixture body\",\"path\":\"service_notes.md\"}"
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &journal,
        "Service Notes"
    ));

    let mut text_only_journal = crate::task_journal::TaskJournal::for_task(
        "task-wrapped-read-heading-text-only",
        "ask",
        "read it",
    );
    text_only_journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "text": "{\"action\":\"read_text_range\",\"excerpt\":\"1|# Release Checklist\\n2|\\n3|fixture body\",\"path\":\"release_checklist.md\"}"
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route,
        &text_only_journal,
        "Release Checklist"
    ));
}

#[test]
fn existence_with_path_answer_grounded_by_existing_path_fact_skips_llm_verifier() {
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
fn directory_purpose_summary_accepts_wrapped_inventory_largest_and_content_excerpt() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryPurposeSummary;
    route.output_contract.requires_content_evidence = true;
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-dir-purpose-wrapped", "ask", "summarize");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "extra": {
                    "action": "inventory_dir",
                    "entries": [
                        {
                            "kind": "file",
                            "name": "contract_repair_judge.schema.json",
                            "path": "prompts/schemas/contract_repair_judge.schema.json",
                            "size_bytes": 6754
                        },
                        {
                            "kind": "file",
                            "name": "intent_normalizer.schema.json",
                            "path": "prompts/schemas/intent_normalizer.schema.json",
                            "size_bytes": 14775
                        }
                    ],
                    "names": [
                        "contract_repair_judge.schema.json",
                        "intent_normalizer.schema.json"
                    ],
                    "size_summary": {
                        "largest_file": {
                            "kind": "file",
                            "name": "intent_normalizer.schema.json",
                            "path": "prompts/schemas/intent_normalizer.schema.json",
                            "size_bytes": 14775
                        }
                    }
                },
                "text": "{\"action\":\"inventory_dir\"}"
            })
            .to_string(),
        ));
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_2",
            "fs_basic",
            json!({
                "extra": {
                    "action": "read_range",
                    "path": "prompts/schemas/intent_normalizer.schema.json",
                    "excerpt": "1|{\n2|  \"title\": \"IntentNormalizerOut\",\n3|  \"description\": \"Schema for the JSON object returned by the unified intent normalizer prompt.\""
                },
                "text": "{\"action\":\"read_range\"}"
            })
            .to_string(),
        ));

    let candidate = concat!(
        "documentation.files.count=2; documentation.files=contract_repair_judge.schema.json, intent_normalizer.schema.json\n",
        "largest.name=intent_normalizer.schema.json; largest.path=prompts/schemas/intent_normalizer.schema.json; largest.size_bytes=14775\n",
        "content_excerpt=IntentNormalizerOut Schema for the JSON object returned by the unified intent normalizer prompt."
    );

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, candidate
    ));
    assert!(structural_satisfaction_can_skip_verifier(
        &route, &journal, candidate
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "largest.name=contract_repair_judge.schema.json; largest.size_bytes=6754"
    ));
}

#[test]
fn existence_with_path_answer_grounded_by_missing_path_fact_skips_llm_verifier() {
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
    let route = route_with_mode();
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
    let route = route_with_mode();
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

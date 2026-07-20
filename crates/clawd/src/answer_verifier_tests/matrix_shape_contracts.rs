use super::*;

#[test]
fn matrix_scalar_shape_requires_plain_scalar_answer() {
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
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
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.selection.structured_field_selector = Some("path".to_string());
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
fn matrix_strict_list_shape_rejects_unobserved_items() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.selection.list_selector.target_kind =
        crate::OutputScalarCountTargetKind::File;
    route
        .output_contract
        .selection
        .list_selector
        .target_kind_specified = true;
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
fn matrix_single_path_shape_requires_plain_grounded_path() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.selection.structured_field_selector = Some("path".to_string());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-matrix-path", "ask", "write report");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output_excerpt: Some(
                json!({
                    "action": "write_text",
                    "path": "/tmp/rustclaw/report.md"
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
        "/tmp/rustclaw/report.md"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "Path: /tmp/rustclaw/report.md"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "The report is /tmp/rustclaw/report.md"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "/tmp/rustclaw/missing.zip"
    ));
}

#[test]
fn matrix_scalar_shape_uses_observed_evidence_map_values() {
    let mut route = route_with_mode();
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

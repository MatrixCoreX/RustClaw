use super::*;

#[test]
fn filesystem_count_entries_capability_ref_verifies_scalar_without_semantic_kind() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.resolved_intent = "capability_ref=filesystem.count_entries".to_string();
    route.route_reason = "capability_ref=filesystem.count_entries".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-count-entries-capability",
        "ask",
        "capability_ref=filesystem.count_entries",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({"action": "count_entries", "count": 3, "path": "."}).to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "3"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route, &journal, "count: 3"
    ));
}

#[test]
fn system_runtime_status_capability_ref_verifies_scalar_without_semantic_kind() {
    let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
    route.resolved_intent = "capability_ref=system.runtime_status".to_string();
    route.route_reason = "capability_ref=system.runtime_status".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;

    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-runtime-status-capability",
        "ask",
        "capability_ref=system.runtime_status",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "system_basic",
            json!({
                "action": "runtime_status",
                "kind": "current_user",
                "value": "guagua",
                "field_value": "guagua"
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "guagua"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "current_user: guagua"
    ));
}

use super::*;

#[test]
fn planner_scalar_count_contract_verifies_scalar() {
    let mut route = route_with_mode();
    route.request_text = "capability_ref=filesystem.count_entries".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.selection.structured_field_selector = Some("count".to_string());

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
fn planner_scalar_runtime_contract_verifies_scalar() {
    let mut route = route_with_mode();
    route.request_text = "capability_ref=system.runtime_status".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
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

#[test]
fn planner_scalar_config_field_contract_verifies_scalar() {
    let mut route = route_with_mode();
    route.request_text = "capability_ref=config.read_field".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-config-read-field-capability",
        "ask",
        "capability_ref=config.read_field",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "config_basic",
            json!({
                "action": "read_field",
                "path": "configs/config.toml",
                "field_path": "server.port",
                "exists": true,
                "value": 8080,
                "field_value": 8080
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "8080"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "server.port: 8080"
    ));
}

#[test]
fn planner_scalar_extracted_field_contract_verifies_scalar() {
    let mut route = route_with_mode();
    route.request_text = "capability_ref=system_basic.extract_field".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-system-extract-field-capability",
        "ask",
        "capability_ref=system_basic.extract_field",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "system_basic",
            json!({
                "action": "extract_field",
                "path": "Cargo.toml",
                "field_path": "package.version",
                "exists": true,
                "value": "0.1.8",
                "field_value": "0.1.8"
            })
            .to_string(),
        ));

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, "0.1.8"
    ));
    assert!(!structurally_satisfies_answer_contract(
        &route,
        &journal,
        "package.version: 0.1.8"
    ));
}

use super::{answer_contract, machine_status_visible_output_format_gap, route_result};
use crate::{OutputLocatorKind, OutputResponseShape, OutputSemanticKind};
use serde_json::json;

#[test]
fn service_status_machine_token_visible_answer_records_output_format_gap() {
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.locator_kind = OutputLocatorKind::None;
    route.requires_content_evidence = true;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-service-token", "ask", "status");
    journal.record_output_contract(&route.clone());
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "process_basic",
            json!({
                "extra": {
                    "action": "ps",
                    "filter": "telegramd",
                    "process_count": 0,
                    "running": false,
                    "status": "not_running"
                },
                "text": "ignored user-visible fallback"
            })
            .to_string(),
        ));

    let verifier =
        machine_status_visible_output_format_gap(&answer_contract(&route), &journal, "not_running")
            .expect("visible machine status token should be an output-format gap");

    assert_eq!(verifier.missing_evidence_fields, vec!["output_format"]);
    assert_eq!(
        verifier.answer_incomplete_reason,
        "machine_status_token_visible"
    );
    assert!(verifier.should_retry);
}

#[test]
fn scalar_service_status_contract_keeps_exact_machine_token_answer() {
    let mut route = route_result(OutputResponseShape::Scalar);
    route.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.locator_kind = OutputLocatorKind::None;

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-service-scalar", "ask", "status");
    journal.record_output_contract(&route.clone());
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "process_basic",
            json!({
                "extra": {
                    "status": "not_running"
                },
                "text": "ignored user-visible fallback"
            })
            .to_string(),
        ));

    assert!(machine_status_visible_output_format_gap(
        &answer_contract(&route),
        &journal,
        "not_running"
    )
    .is_none());
}

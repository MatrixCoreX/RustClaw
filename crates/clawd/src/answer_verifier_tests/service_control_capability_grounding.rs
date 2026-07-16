use super::*;

#[test]
fn service_status_contract_grounds_port_answer() {
    let mut route = route_with_mode();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-service-control-ports-capability-ref",
        "ask",
        "port.number port.local",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_ports",
            "process_basic",
            "port.count=2\nport[0].number=22\nport[0].local=0.0.0.0:22\nport[1].number=80\nport[1].local=0.0.0.0:80",
        ));
    let candidate = "\
| port | bind |
| --- | --- |
| 22 | 0.0.0.0:22 |
| 80 | 0.0.0.0:80 |";

    assert!(structurally_satisfies_answer_contract(
        &route, &journal, candidate
    ));
}

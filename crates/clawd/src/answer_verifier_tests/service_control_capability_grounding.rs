use super::*;

#[test]
fn service_control_capability_ref_port_answer_is_grounded_without_semantic_kind() {
    let mut route = route_with_mode(crate::AskMode::act_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.route_reason = "capability_ref=service_control.status".to_string();
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

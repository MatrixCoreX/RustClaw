use super::*;

#[test]
fn scalar_service_status_without_machine_health_capability_defers_to_planner() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "ServiceStatus semantic marker alone must not choose a registry action before the planner"
    );
}

#[test]
fn service_status_task_id_without_task_control_capability_defers_to_planner() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let task_id = "00000000-0000-4000-8000-000000000010";
    route.resolved_intent = format!("task_id={task_id}");

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "task_id text without capability_ref must remain planner-owned"
    );
}

#[test]
fn command_output_task_id_without_task_control_capability_defers_to_planner() {
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    let task_id = "00000000-0000-4000-8000-000000000011";
    route.resolved_intent = format!("task_id={task_id}");

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "task lifecycle text without capability_ref must not create a deterministic task_control action"
    );
}

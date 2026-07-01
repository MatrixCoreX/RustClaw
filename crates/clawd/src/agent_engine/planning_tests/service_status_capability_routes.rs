use super::*;

#[test]
fn scalar_service_status_without_machine_health_capability_defers_to_planner() {
    let state = test_state_with_enabled_skills(&["health_check", "process_basic"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "return one runtime scalar",
        Some(&route),
        &loop_state,
        "current runtime scalar",
    );

    assert!(plan.is_none());
}

#[test]
fn service_status_task_id_without_task_control_capability_defers_to_planner() {
    let state = test_state_with_enabled_skills(&["task_control"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    let task_id = "00000000-0000-4000-8000-000000000010";
    route.resolved_intent = format!("task_id={task_id}");
    let loop_state = LoopState::new(1);

    let plan = service_status_deterministic_plan_result(
        &state,
        "observe task lifecycle",
        Some(&route),
        &loop_state,
        &format!("query task {task_id}"),
    );

    assert!(plan.is_none());
}

#[test]
fn command_output_task_id_without_task_control_capability_defers_to_planner() {
    let state = test_state_with_enabled_skills(&["task_control"]);
    let mut route = base_route_result();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::CommandOutputSummary;
    let task_id = "00000000-0000-4000-8000-000000000011";
    route.resolved_intent = format!("task_id={task_id}");
    let loop_state = LoopState::new(1);

    let plan = task_control_get_deterministic_plan_result(
        &state,
        "observe task lifecycle fields",
        Some(&route),
        &loop_state,
        &format!("task_id={task_id} data.lifecycle.can_poll"),
    );

    assert!(plan.is_none());
}

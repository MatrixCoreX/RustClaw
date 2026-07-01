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

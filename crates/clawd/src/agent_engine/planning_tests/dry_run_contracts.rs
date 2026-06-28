use super::*;

#[test]
fn answer_verifier_contract_dry_run_returns_machine_contract_fields() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run required_evidence missing_evidence_fields contract_boundary".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: required_evidence missing_evidence_fields contract_boundary",
        Some(&route),
        &loop_state,
    )
    .expect("answer verifier dry-run contract should be deterministic");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value.get("semantic_kind").and_then(Value::as_str),
        Some("answer_verifier_contract_dry_run")
    );
    assert!(value.get("required_evidence").is_some());
    assert!(value.get("missing_evidence_fields").is_some());
    assert!(value.get("contract_boundary").is_some());
}

#[test]
fn async_job_contract_dry_run_exposes_lifecycle_checkpoint_fields() {
    let mut route = base_route_result();
    route.route_reason = "dry_run pending_async_job poll_async_job".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: pending_async_job checkpoint_id poll_ref next_check_after can_cancel",
        Some(&route),
        &loop_state,
    )
    .expect("async job dry-run contract should be deterministic");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value
            .pointer("/task_lifecycle/checkpoint_id")
            .and_then(Value::as_str),
        Some("opaque_checkpoint_id")
    );
    assert_eq!(
        value
            .pointer("/task_lifecycle/poll_ref")
            .and_then(Value::as_str),
        Some("adapter_result.job_id")
    );
    assert_eq!(
        value
            .pointer("/task_lifecycle/can_cancel")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn local_process_cancel_dry_run_prefers_local_process_adapter_contract() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run cancel_ref adapter_kind=local_process_poll status=cancelled terminal_projection"
            .to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: cancel_ref adapter_kind=local_process_poll status=cancelled terminal_projection",
        Some(&route),
        &loop_state,
    )
    .expect("local process cancel dry-run contract should be deterministic");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value.get("adapter_kind").and_then(Value::as_str),
        Some("local_process_poll")
    );
    assert_eq!(
        value.get("status").and_then(Value::as_str),
        Some("cancelled")
    );
    assert_eq!(
        value
            .pointer("/terminal_projection/state")
            .and_then(Value::as_str),
        Some("cancelled")
    );
}

#[test]
fn observed_output_projection_dry_run_prefers_projection_contract() {
    let mut route = base_route_result();
    route.route_reason =
        "dry_run observed-output scalar list path json field status artifact_refs".to_string();
    let loop_state = LoopState::new(1);

    let plan = structured_dry_run_response_deterministic_plan_result(
        "Dry run only: return observed-output scalar list path JSON field status artifact_refs",
        Some(&route),
        &loop_state,
    )
    .expect("observed-output projection dry-run contract should be deterministic");

    let action = plan.steps[0].to_agent_action().expect("agent action");
    let AgentAction::Respond { content } = action else {
        panic!("expected respond action, got {action:?}");
    };
    let value: Value = serde_json::from_str(&content).expect("json response");
    assert_eq!(
        value.get("semantic_kind").and_then(Value::as_str),
        Some("observed_output_projection_dry_run")
    );
    let families = value
        .get("families")
        .and_then(Value::as_array)
        .expect("families");
    assert!(families
        .iter()
        .any(|item| item.as_str() == Some("artifact_refs")));
}

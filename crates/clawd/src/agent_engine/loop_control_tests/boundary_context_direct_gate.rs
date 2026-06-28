use super::super::LoopBudgetProfile;
use super::*;

#[test]
fn boundary_context_classifies_pre_agent_gate_machine_summary() {
    let policy = test_policy();
    let mut route = route_result(OutputResponseShape::Scalar);
    route.route_reason =
        "clarify_reason_code:missing_read_target; direct_answer_gate_unbound_deictic_clarify"
            .to_string();
    route.output_contract.requires_content_evidence = true;

    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext {
            fuzzy_locator_suggestions: vec!["README.md".to_string()],
            ..AgentRunContext::default()
        }),
        Some(&route),
        LoopBudgetProfile::FastRead,
    );

    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/post_route_policy/boundary_class")
            .and_then(serde_json::Value::as_str),
        Some("locator_fuzzy_candidates")
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/post_route_policy/ownership_class")
            .and_then(serde_json::Value::as_str),
        Some("boundary_machine_check")
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/post_route_policy/boundary_allowed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/post_route_policy/semantic_migration_target")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/post_route_policy/fuzzy_locator_suggestion_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/observed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/observation_class")
            .and_then(serde_json::Value::as_str),
        Some("legacy_gate_observed")
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/boundary_class")
            .and_then(serde_json::Value::as_str),
        Some("locator_binding_fallback")
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/boundary_allowed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn boundary_context_marks_direct_answer_contract_execute_as_boundary_owned() {
    let policy = test_policy();
    let mut route = route_result(OutputResponseShape::Scalar);
    route.route_reason = "direct_answer_gate_contract_execute:structured contract".to_string();
    route.output_contract.requires_content_evidence = true;

    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        LoopBudgetProfile::FastRead,
    );

    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/boundary_class")
            .and_then(serde_json::Value::as_str),
        Some("contract_execution_boundary")
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/ownership_class")
            .and_then(serde_json::Value::as_str),
        Some("contract_boundary")
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/boundary_allowed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/semantic_migration_target")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );
}

#[test]
fn boundary_context_treats_bare_direct_answer_execute_as_agent_loop_activation() {
    let policy = test_policy();
    let mut route = route_result(OutputResponseShape::Scalar);
    route.route_reason = "direct_answer_gate_execute:legacy semantic promotion".to_string();
    route.output_contract.requires_content_evidence = true;

    let boundary = boundary_context_snapshot_json(
        &test_task(),
        &policy,
        Some(&AgentRunContext::default()),
        Some(&route),
        LoopBudgetProfile::FastRead,
    );

    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/boundary_class")
            .and_then(serde_json::Value::as_str),
        Some("agent_loop_activation_boundary")
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/ownership_class")
            .and_then(serde_json::Value::as_str),
        Some("agent_loop_activation")
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/boundary_allowed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        boundary
            .pointer("/pre_agent_gates/direct_answer_gate/semantic_migration_target")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );
}

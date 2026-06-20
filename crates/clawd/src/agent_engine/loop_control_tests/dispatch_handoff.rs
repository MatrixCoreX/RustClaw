use super::*;

#[test]
fn ordinary_clarify_handoff_records_dispatch_owner_attribution() {
    let mut policy = test_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::Shadow;
    let mut route = route_result(OutputResponseShape::Free);
    route.route_reason = "ordinary_clarify_deferred_to_agent_loop".to_string();
    let task = test_task();
    let mut loop_state = LoopState::new(1);

    maybe_record_agent_decides_shadow_attribution(
        &policy,
        &task,
        Some(&AgentRunContext::default()),
        Some(&route),
        super::super::LoopBudgetProfile::General,
        &mut loop_state,
    );

    assert_eq!(loop_state.rollout_attribution.len(), 2);
    let handoff = &loop_state.rollout_attribution[0];
    assert_eq!(handoff.switch_name, "semantic_route_authority");
    assert_eq!(
        handoff.event.as_str(),
        "ordinary_clarify_deferred_to_agent_loop"
    );
    assert_eq!(
        handoff.boundary_context.as_ref().and_then(|value| {
            value
                .pointer("/old_owner")
                .and_then(serde_json::Value::as_str)
        }),
        Some("legacy_pre_agent_semantic_clarify")
    );
    assert_eq!(
        handoff.boundary_context.as_ref().and_then(|value| {
            value
                .pointer("/new_owner")
                .and_then(serde_json::Value::as_str)
        }),
        Some("agent_loop_terminal_clarify")
    );
    assert_eq!(
        handoff.boundary_context.as_ref().and_then(|value| {
            value
                .pointer("/rollback_token")
                .and_then(serde_json::Value::as_str)
        }),
        Some("semantic_route_authority:legacy_pre_agent")
    );
    assert_eq!(
        loop_state.rollout_attribution[1].event.as_str(),
        "agent_decides_shadow_snapshot"
    );
}

#[test]
fn resume_discussion_handoff_records_dispatch_owner_attribution() {
    let mut policy = test_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::Shadow;
    let mut route = route_result(OutputResponseShape::Free);
    route.route_reason = "resume_discussion_requires_agent_loop".to_string();
    let task = test_task();
    let mut loop_state = LoopState::new(1);

    maybe_record_agent_decides_shadow_attribution(
        &policy,
        &task,
        Some(&AgentRunContext::default()),
        Some(&route),
        super::super::LoopBudgetProfile::General,
        &mut loop_state,
    );

    assert_eq!(loop_state.rollout_attribution.len(), 2);
    let handoff = &loop_state.rollout_attribution[0];
    assert_eq!(
        handoff.event.as_str(),
        "resume_discussion_requires_agent_loop"
    );
    assert_eq!(
        handoff.boundary_context.as_ref().and_then(|value| {
            value
                .pointer("/old_owner")
                .and_then(serde_json::Value::as_str)
        }),
        Some("legacy_pre_agent_resume_discussion")
    );
    assert_eq!(
        handoff.boundary_context.as_ref().and_then(|value| {
            value
                .pointer("/new_owner")
                .and_then(serde_json::Value::as_str)
        }),
        Some("agent_loop_resume_discussion")
    );
    assert_eq!(
        handoff.boundary_context.as_ref().and_then(|value| {
            value
                .pointer("/chosen_path")
                .and_then(serde_json::Value::as_str)
        }),
        Some("agent_loop_chat_wrapped_resume")
    );
}

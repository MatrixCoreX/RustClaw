use super::build_agent_run_context_from_prepared_flow;

#[test]
fn planner_context_keeps_raw_request_without_pre_planner_route() {
    let task = crate::ClaimedTask {
        task_id: "task-runtime-context".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let flow = super::PreparedAskFlow {
        context_bundle_summary: "summary".to_string(),
        memory_trace: None,
        turn_boundary_envelope:
            crate::turn_boundary_envelope::TurnBoundaryEnvelope::from_claimed_task(
                &task,
                &serde_json::json!({}),
                "raw request",
                crate::turn_boundary_envelope::TurnInputMaterialization::RawText,
                None,
                false,
                false,
            ),
        auto_locator_path: None,
        planner_user_request: "raw request".to_string(),
        resolved_prompt_for_execution: "raw request".to_string(),
        prompt_with_memory_for_execution: "memory context\nraw request".to_string(),
        recent_execution_context: "<none>".to_string(),
        session_alias_bindings: Vec::new(),
    };

    let context = build_agent_run_context_from_prepared_flow(&flow);
    assert_eq!(
        context.original_user_request.as_deref(),
        Some("raw request")
    );
    assert_eq!(context.user_request.as_deref(), Some("raw request"));
    assert!(context.route_result.is_none());
    assert!(context.turn_analysis.is_none());
    assert!(context.auto_locator_path.is_none());
}

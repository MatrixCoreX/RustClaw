use super::build_agent_run_context_from_prepared_flow;

#[test]
fn planner_context_keeps_neutral_route_and_raw_request() {
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
        route_result: crate::RouteResult {
            resolved_intent: "raw request".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "agent_loop_semantic_authority".to_string(),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        },
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
    assert_eq!(
        context
            .route_result
            .as_ref()
            .map(|route| route.route_reason.as_str()),
        Some("agent_loop_semantic_authority")
    );
    assert!(context.turn_analysis.is_none());
    assert!(context.auto_locator_path.is_none());
}

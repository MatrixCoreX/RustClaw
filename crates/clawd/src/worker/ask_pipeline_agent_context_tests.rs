use super::{build_agent_run_context_from_prepared_flow, PreparedAskFlow};

fn base_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "inspect workspace".to_string(),
        needs_clarify: false,
        route_reason: "test_route".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: "RustClaw".to_string(),
        output_contract: crate::IntentOutputContract::default(),
    }
}

fn prepared_flow_with_context() -> PreparedAskFlow {
    PreparedAskFlow {
        context_bundle_summary: "context-summary".to_string(),
        memory_trace: None,
        route_result: base_route(),
        execution_recipe_hint: None,
        execution_recipe_plan_hint: None,
        turn_analysis: None,
        clarify_fallback_source: None,
        auto_locator_path: Some("/tmp/workspace/README.md".to_string()),
        has_authoritative_deictic_anchor: true,
        chat_prompt_context: "chat prompt".to_string(),
        resolved_prompt_for_execution: "resolved execution prompt".to_string(),
        prompt_with_memory_for_execution: "memory + resolved execution prompt".to_string(),
        memory_context_for_execution: "memory facts".to_string(),
        semantic_answer_candidate_draft: Some("candidate draft".to_string()),
        recent_execution_context: "recent execution facts".to_string(),
        session_alias_bindings: Vec::new(),
        agent_mode: true,
        ask_mode: crate::AskMode::planner_execute_plain(),
        clarify_reason: String::new(),
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        fuzzy_locator_suggestions: vec!["README.md".to_string()],
        should_route_schedule_direct: false,
    }
}

#[test]
fn prepared_ask_flow_builds_agent_run_context_for_replay_boundary() {
    let prepared_flow = prepared_flow_with_context();
    let ctx = build_agent_run_context_from_prepared_flow("raw user request", &prepared_flow);

    assert_eq!(
        ctx.original_user_request.as_deref(),
        Some("raw user request")
    );
    assert_eq!(
        ctx.user_request.as_deref(),
        Some("resolved execution prompt")
    );
    assert_eq!(
        ctx.context_bundle_summary.as_deref(),
        Some("context-summary")
    );
    assert_eq!(
        ctx.auto_locator_path.as_deref(),
        Some("/tmp/workspace/README.md")
    );
    assert_eq!(
        ctx.memory_context_for_execution.as_deref(),
        Some("memory facts")
    );
    assert_eq!(
        ctx.cross_turn_recent_execution_context.as_deref(),
        Some("recent execution facts")
    );
    assert_eq!(
        ctx.semantic_answer_candidate_draft.as_deref(),
        Some("candidate draft")
    );
    assert!(ctx.has_authoritative_deictic_anchor);
    assert_eq!(ctx.fuzzy_locator_suggestions, vec!["README.md"]);
    assert_eq!(
        ctx.route_result
            .as_ref()
            .map(|route| route.gate_kind().as_str()),
        Some("execute")
    );
}

#[test]
fn prepared_ask_flow_omits_empty_memory_and_recent_context() {
    let mut prepared_flow = prepared_flow_with_context();
    prepared_flow.memory_context_for_execution = "<none>".to_string();
    prepared_flow.recent_execution_context = "   ".to_string();

    let ctx = build_agent_run_context_from_prepared_flow("raw user request", &prepared_flow);

    assert_eq!(ctx.memory_context_for_execution, None);
    assert_eq!(ctx.cross_turn_recent_execution_context, None);
}

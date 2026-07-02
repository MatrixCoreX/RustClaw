use super::{
    agent_loop_boundary_observations_block, agent_loop_default_context,
    build_agent_run_context_from_prepared_flow, PreparedAskFlow,
};

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
        resolved_prompt_for_execution: "resolved execution prompt".to_string(),
        prompt_with_memory_for_execution: "memory + resolved execution prompt".to_string(),
        recent_execution_context: "recent execution facts".to_string(),
        session_alias_bindings: Vec::new(),
        ask_mode: crate::AskMode::planner_execute_plain(),
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
        ctx.cross_turn_recent_execution_context.as_deref(),
        Some("recent execution facts")
    );
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
    prepared_flow.recent_execution_context = "   ".to_string();

    let ctx = build_agent_run_context_from_prepared_flow("raw user request", &prepared_flow);

    assert_eq!(ctx.cross_turn_recent_execution_context, None);
}

#[test]
fn agent_loop_default_context_demotes_post_route_clarify_to_loop_context() {
    let mut route = base_route();
    route.needs_clarify = true;
    route.clarify_question = "missing target".to_string();
    route.set_clarify_gate();
    route.route_reason = "post_route_locator_guard".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let loop_ctx = agent_loop_default_context(Some(ctx)).expect("loop context");
    let route = loop_ctx.route_result.expect("route context");

    assert!(!route.needs_clarify);
    assert!(route.clarify_question.is_empty());
    assert_eq!(route.gate_kind(), crate::RouteGateKind::Execute);
    assert!(route.route_reason.contains("agent_loop_default_entry"));
}

#[test]
fn boundary_observation_block_filters_natural_language_route_reason() {
    let mut route = base_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.route_reason =
        "用户自然语言原因; machine_boundary_code; MixedCaseCode; another.machine_code".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.requires_content_evidence = true;

    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: Some("/tmp/workspace/README.md".to_string()),
        auto_locator_hint: None,
        auto_locator_resolved_direct: true,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_locator_boundary",
            crate::post_route_policy::PostRoutePolicyOutcome::BoundaryClarify,
        ),
    };
    let session_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "docs alias".to_string(),
                target: "/tmp/workspace/docs".to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some("/tmp/workspace/notes.md".to_string()),
            ordered_entries: vec!["one.md".to_string(), "two.md".to_string()],
            observed_entry_count: Some(2),
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };

    let state = crate::AppState::test_default_with_fixture_provider();
    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &session_snapshot,
        None,
        "inspect /tmp/workspace/README.md",
        "resolved capability_ref=kb.list_namespaces",
        &[],
    )
    .expect("observation block");

    assert!(block.contains("AGENT_LOOP_BOUNDARY_OBSERVATIONS"));
    assert!(block.contains("machine_boundary_code"));
    assert!(block.contains("another.machine_code"));
    assert!(block.contains("/tmp/workspace/README.md"));
    assert!(block.contains("current_request_locator"));
    assert!(block.contains("explicit_locator_hints"));
    assert!(block.contains("registry_capability_contract"));
    assert!(block.contains("kb.list_namespaces"));
    assert!(block.contains("docs alias"));
    assert!(block.contains("/tmp/workspace/docs"));
    assert!(block.contains("active_observed_facts"));
    assert!(block.contains("/tmp/workspace/notes.md"));
    assert!(!block.contains("route_gate_kind"));
    assert!(!block.contains("用户自然语言原因"));
    assert!(!block.contains("MixedCaseCode"));
}

#[test]
fn boundary_observation_block_omits_legacy_route_trace_only_reason() {
    let mut route = base_route();
    route.route_reason = "executionless_finalize_trace_plain".to_string();
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_no_change",
            crate::post_route_policy::PostRoutePolicyOutcome::NoChange,
        ),
    };
    let state = crate::AppState::test_default_with_fixture_provider();

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        None,
        "answer from current runtime context",
        "answer from current runtime context",
        &[],
    );

    assert!(block.is_none());
}

#[test]
fn boundary_observation_block_exports_runtime_session_state_for_status_query() {
    let route = base_route();
    let post_route = crate::post_route_policy::PostRoutePolicyResult {
        execution_route_result: route,
        auto_locator_path: None,
        auto_locator_hint: None,
        auto_locator_resolved_direct: false,
        fuzzy_locator_suggestions: Vec::new(),
        missing_locator_for_path_scoped_content: false,
        clarify_reason_kind: crate::post_route_policy::ClarifyReasonKind::RouteReasonText,
        gate_record: crate::post_route_policy::PostRouteGateRecord::new(
            "post_route_no_change",
            crate::post_route_policy::PostRoutePolicyOutcome::NoChange,
        ),
    };
    let state = crate::AppState::test_default_with_fixture_provider();
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::StatusQuery),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "runtime_status_query": {"kind": "current_user_boundary", "scope": "session"}
        })),
        attachment_processing_required: false,
    };

    let block = agent_loop_boundary_observations_block(
        &state,
        &post_route,
        &crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        },
        Some(&turn_analysis),
        "answer from current runtime context",
        "answer from current runtime context",
        &[],
    )
    .expect("runtime status query should export boundary state");

    assert!(block.contains("\"runtime_session_state\""));
    assert!(block.contains("\"active_task_present\":false"));
    assert!(block.contains("\"pending_user_boundary_present\":false"));
}

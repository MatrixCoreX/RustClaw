use super::{
    skill_execution_preflight::evidence_policy_action_policy_error, tests::test_state, LoopState,
};

#[test]
fn evidence_policy_preflight_rejects_tools_for_executionless_terminal_boundary() {
    let state = test_state();
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "executionless terminal boundary".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "boundary_only; executionless_finalize_trace_plain".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let mut loop_state = LoopState::new(2);
    loop_state.route_policy_context = Some(route);
    let args = serde_json::json!({"action": "read_text_range", "path": "plan/current.md"});

    let err =
        evidence_policy_action_policy_error(&state, &loop_state, "fs_basic", &args, "call_skill")
            .expect("executionless terminal boundary must reject tool calls");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("executionless preflight error should be structured");
    assert_eq!(parsed.error_kind, "contract_action_rejected");
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason_code"))
            .and_then(|value| value.as_str()),
        Some("executionless_boundary_tool_call_blocked")
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.pointer("/permission_decision/owner_layer"))
            .and_then(|value| value.as_str()),
        Some("executionless_boundary_preflight")
    );
}

#[test]
fn evidence_policy_preflight_allows_verified_observe_tool_for_executionless_boundary() {
    let state = test_state();
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "executionless terminal boundary".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "boundary_only; executionless_finalize_trace_plain".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let mut loop_state = LoopState::new(2);
    loop_state.route_policy_context = Some(route);
    loop_state.verified_action_window_active = true;
    let args = serde_json::json!({
        "action": "inventory_dir",
        "path": "/home/guagua/rustclaw",
        "names_only": true
    });

    assert!(
        evidence_policy_action_policy_error(&state, &loop_state, "system_basic", &args, "call_skill")
            .is_none(),
        "verifier-approved read-only observations should not be denied by the legacy executionless guard"
    );
}

#[test]
fn evidence_policy_preflight_allows_verified_observe_capability_for_executionless_boundary() {
    let state = test_state();
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "executionless terminal boundary".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "boundary_only; executionless_finalize_trace_plain".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let mut loop_state = LoopState::new(2);
    loop_state.route_policy_context = Some(route);
    loop_state.verified_action_window_active = true;
    let args = serde_json::json!({
        "action": "inventory_dir",
        "path": "/home/guagua/rustclaw",
        "include_hidden": true,
        "names_only": true
    });

    assert!(
        evidence_policy_action_policy_error(
            &state,
            &loop_state,
            "system_basic",
            &args,
            "call_capability"
        )
        .is_none(),
        "verifier-approved observe call_capability should not be denied by the legacy executionless guard"
    );
}

#[test]
fn evidence_policy_preflight_allows_literal_run_cmd_for_executionless_boundary() {
    let state = test_state();
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "executionless terminal boundary".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "boundary_only; executionless_finalize_trace_plain".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let mut loop_state = LoopState::new(2);
    loop_state.route_policy_context = Some(route);
    let args = serde_json::json!({
        "command": "pwd; whoami; hostname",
        super::super::CLAWD_LITERAL_COMMAND_ARG: true
    });

    assert!(
        evidence_policy_action_policy_error(&state, &loop_state, "run_cmd", &args, "call_skill").is_none(),
        "literal run_cmd is planner-selected execution evidence and should not be blocked by the executionless answer guard"
    );
}

#[test]
fn evidence_policy_preflight_allows_plain_execute_gate_despite_executionless_marker() {
    let state = test_state();
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "plain execute with stale executionless marker".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "executionless_finalize_trace_plain".to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Medium,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let mut loop_state = LoopState::new(2);
    loop_state.route_policy_context = Some(route);
    let args = serde_json::json!({
        "action": "read_text_range",
        "path": "/home/guagua/rustclaw/run/nl_eval_tmp/codex_cli_continuous_20260711_new/calc_core.py",
        "mode": "head",
        "n": 200,
    });

    assert!(
        evidence_policy_action_policy_error(&state, &loop_state, "fs_basic", &args, "call_skill").is_none(),
        "plain execute gate should let planner-selected observe actions reach normal policy/verifier even if a stale executionless marker is present"
    );
}

#[test]
fn evidence_policy_preflight_allows_agent_loop_execution_despite_executionless_marker() {
    let state = test_state();
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "agent-loop execution with stale executionless marker".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason:
            "executionless_finalize_trace_plain; executable_contract_preserved_for_agent_loop"
                .to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Medium,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let mut loop_state = LoopState::new(2);
    loop_state.route_policy_context = Some(route);
    let args = serde_json::json!({
        "action": "write_text",
        "path": "/home/guagua/rustclaw/run/nl_eval_tmp/codex_cli_continuous_20260711_new/calc_core.py",
        "content": "def mul(a, b):\n    return a * b\n"
    });

    assert!(
        evidence_policy_action_policy_error(&state, &loop_state, "fs_basic", &args, "call_skill").is_none(),
        "agent-loop execution markers should route planner-selected writes through verifier/policy instead of executionless terminal guard"
    );
}

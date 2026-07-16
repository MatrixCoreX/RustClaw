use super::*;

#[test]
fn structured_respond_clarify_step_marks_loop_pending_user_input() {
    let question = "Which file should I read?";
    let plan = plan_result_with_raw_and_steps(
        "{}",
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": question,
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_locator",
                "missing_slot": "locator",
                "message_key": "clawd.clarify.locator_required",
                "field_path": "output_contract.locator_hint",
                "locator_kind": "path"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut loop_state = LoopState::new(2);
    let outcome = apply_structured_respond_clarify_to_loop_state(&mut loop_state, &intent);

    assert!(loop_state.pending_user_input_required);
    assert_eq!(loop_state.delivery_messages, vec![question.to_string()]);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some(question)
    );
    assert_eq!(outcome.executed_actions, 0);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("structured_respond_clarify")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.terminal_intent")
            .map(String::as_str),
        Some("clarify")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.missing_slot")
            .map(String::as_str),
        Some("locator")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.message_key")
            .map(String::as_str),
        Some("clawd.clarify.locator_required")
    );
}

#[test]
fn route_owned_respond_only_clarify_marks_loop_pending_user_input() {
    let question = "Which file should I read?";
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.needs_clarify = true;
    route.clarify_question = question.to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.requires_content_evidence = false;
    let actions = vec![AgentAction::Respond {
        content: question.to_string(),
    }];
    let intent =
        structured_respond_terminal_intent_from_route_owned_clarify(Some(&route), &actions)
            .expect("route clarify intent");
    let mut loop_state = LoopState::new(1);

    let outcome = apply_structured_respond_clarify_to_loop_state(&mut loop_state, &intent);

    assert!(loop_state.pending_user_input_required);
    assert_eq!(loop_state.delivery_messages, vec![question.to_string()]);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("structured_respond_clarify")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.locator_kind")
            .map(String::as_str),
        Some("path")
    );
}

#[test]
fn boundary_observation_tool_action_forces_machine_clarify_without_delivery() {
    let actions = vec![AgentAction::CallCapability {
        capability: "filesystem.find_name".to_string(),
        args: json!({
            "name": "test_calc_core.py"
        }),
    }];
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.boundary_observation_needs_clarify = true;

    let intent = forced_boundary_observation_clarify_intent(&loop_state, &actions)
        .expect("missing boundary referent should force clarify before tools");
    let outcome = apply_structured_respond_clarify_to_loop_state(&mut loop_state, &intent);

    assert!(loop_state.pending_user_input_required);
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.last_user_visible_respond.is_none());
    assert_eq!(outcome.executed_actions, 0);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("structured_respond_clarify")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.clarify_reason_code")
            .map(String::as_str),
        Some("boundary_observation_needs_clarify")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.missing_slot")
            .map(String::as_str),
        Some("referent")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.message_key")
            .map(String::as_str),
        Some("clawd.clarify.missing_referent")
    );
}

#[test]
fn boundary_observation_with_concrete_tool_target_defers_to_agent_loop() {
    let actions = vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: json!({
            "action": "read_text_range",
            "path": "README.md",
            "mode": "head",
            "n": 20
        }),
    }];
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.boundary_observation_needs_clarify = true;

    assert!(
        forced_boundary_observation_clarify_intent(&loop_state, &actions).is_none(),
        "a verifier-approved action with a concrete machine target must execute in the agent loop"
    );
}

#[test]
fn pre_loop_locator_candidate_wraps_plain_respond_as_structured_clarify() {
    let actions = vec![AgentAction::Respond {
        content: "Please provide the file path.".to_string(),
    }];
    let mut loop_state = LoopState::new(2);
    loop_state.output_vars.insert(
        "pre_loop_clarify_candidates".to_string(),
        json!(["background_only_locator"]).to_string(),
    );

    let intent =
        structured_respond_terminal_intent_from_pre_loop_clarify_candidate(&loop_state, &actions)
            .expect("pre-loop locator boundary should create structured clarify intent");
    let outcome = apply_structured_respond_clarify_to_loop_state(&mut loop_state, &intent);

    assert!(loop_state.pending_user_input_required);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("structured_respond_clarify")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.clarify_reason_code")
            .map(String::as_str),
        Some("pre_loop_boundary_clarify_candidate")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.missing_slot")
            .map(String::as_str),
        Some("locator")
    );
}

#[test]
fn side_effect_free_freeform_topic_clarify_replans_without_publishing_question() {
    let plan = plan_result_with_raw_and_steps(
        "{}",
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Please provide more details.",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_topic_scope",
                "missing_slot": "topic_scope"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::Free);
    route.risk_ceiling = RiskCeiling::Low;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let mut loop_state = LoopState::new(2);

    let outcome = try_replan_avoidable_side_effect_free_freeform_clarify(
        &mut loop_state,
        Some(&route),
        &intent,
    )
    .expect("side-effect-free freeform clarify should be recoverable");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(!loop_state.pending_user_input_required);
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.last_user_visible_respond.is_none());
    assert!(loop_state.has_recoverable_failure_context);
    assert_eq!(loop_state.attempt_ledger_entries.len(), 1);
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.avoidable_clarify_replan_used")
            .map(String::as_str),
        Some("true")
    );
}

#[test]
fn repeated_side_effect_free_freeform_clarify_after_replan_is_treated_as_answer() {
    let plan = plan_result_with_raw_and_steps(
        "{}",
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Draft with neutral assumptions.",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_topic",
                "missing_slot": "proposal_topic"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::Free);
    route.needs_clarify = true;
    route.risk_ceiling = RiskCeiling::Low;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let mut loop_state = LoopState::new(3);
    loop_state.pending_user_boundary_present = true;
    loop_state.output_vars.insert(
        "agent_loop.avoidable_clarify_replan_used".to_string(),
        "true".to_string(),
    );

    let outcome = try_replan_avoidable_side_effect_free_freeform_clarify(
        &mut loop_state,
        Some(&route),
        &intent,
    )
    .expect("second optional freeform clarify should be accepted as answer");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("structured_respond_nonblocking_clarify_answer")
    );
    assert!(!loop_state.pending_user_input_required);
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.terminal_intent")
            .map(String::as_str),
        Some("answer")
    );
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("Draft with neutral assumptions.")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.recovered_terminal_intent")
            .map(String::as_str),
        Some("clarify")
    );
}

#[test]
fn active_user_boundary_side_effect_free_freeform_clarify_replans_even_when_route_needs_clarify() {
    let plan = plan_result_with_raw_and_steps(
        "{}",
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Need topic",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_topic",
                "missing_slot": "user_input"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::Free);
    route.needs_clarify = true;
    route.risk_ceiling = RiskCeiling::Low;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let mut loop_state = LoopState::new(2);
    loop_state.pending_user_boundary_present = true;

    let outcome = try_replan_avoidable_side_effect_free_freeform_clarify(
        &mut loop_state,
        Some(&route),
        &intent,
    )
    .expect("active user-input boundary should allow best-effort replan");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(!loop_state.pending_user_input_required);
    assert!(loop_state.delivery_messages.is_empty());
    assert_eq!(loop_state.attempt_ledger_entries.len(), 1);
}

#[test]
fn active_user_boundary_freeform_clarify_replans_despite_stale_content_contract() {
    let plan = plan_result_with_raw_and_steps(
        r#"{"steps":[{"type":"respond","terminal_intent":"clarify","clarify_reason_code":"missing_input","missing_slot":"proposal_topic_scenario","content":"Need topic"}]}"#,
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Need topic",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_input",
                "missing_slot": "proposal_topic_scenario"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::Free);
    route.needs_clarify = false;
    route.risk_ceiling = RiskCeiling::Low;
    route.route_reason = "structured_observation_clarify_repair".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let mut loop_state = LoopState::new(3);
    loop_state.pending_user_boundary_present = true;

    let outcome = try_replan_avoidable_side_effect_free_freeform_clarify(
        &mut loop_state,
        Some(&route),
        &intent,
    )
    .expect("active user-input boundary should allow planner recovery from stale content contract");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(!loop_state.pending_user_input_required);
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.has_recoverable_failure_context);
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.avoidable_clarify_replan_used")
            .map(String::as_str),
        Some("true")
    );
}

#[test]
fn active_user_boundary_freeform_clarify_replans_with_locatorless_content_contract() {
    let plan = plan_result_with_raw_and_steps(
        r#"{"steps":[{"type":"respond","terminal_intent":"clarify","clarify_reason_code":"missing_input","missing_slot":"user_input","content":"Need topic"}]}"#,
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Need topic",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_input",
                "missing_slot": "user_input"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::Strict);
    route.needs_clarify = false;
    route.risk_ceiling = RiskCeiling::Low;
    route.route_reason = "boundary_only; inline_structured_payload_context_execute".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let mut loop_state = LoopState::new(3);
    loop_state.pending_user_boundary_present = true;

    let outcome = try_replan_avoidable_side_effect_free_freeform_clarify(
        &mut loop_state,
        Some(&route),
        &intent,
    )
    .expect("active user-input boundary should recover locatorless stale content contract");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(!loop_state.pending_user_input_required);
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.has_recoverable_failure_context);
}

#[test]
fn active_user_boundary_medium_risk_side_effect_free_freeform_clarify_replans() {
    let plan = plan_result_with_raw_and_steps(
        "{}",
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Need the missing input.",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_input",
                "missing_slot": "user_input"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::Free);
    route.risk_ceiling = RiskCeiling::Medium;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let mut loop_state = LoopState::new(2);
    loop_state.pending_user_boundary_present = true;

    let outcome = try_replan_avoidable_side_effect_free_freeform_clarify(
        &mut loop_state,
        Some(&route),
        &intent,
    )
    .expect("active respond-only follow-up should allow best-effort replan");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(!loop_state.pending_user_input_required);
    assert!(loop_state.delivery_messages.is_empty());
    assert_eq!(loop_state.attempt_ledger_entries.len(), 1);
}

#[test]
fn medium_risk_side_effect_free_content_clarify_replans_without_active_boundary() {
    let plan = plan_result_with_raw_and_steps(
        r#"{"steps":[{"type":"respond","terminal_intent":"clarify","missing_slot":"topic_or_content","content":"Need topic"}]}"#,
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Need topic",
                "terminal_intent": "clarify",
                "missing_slot": "topic_or_content"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::Free);
    route.needs_clarify = false;
    route.risk_ceiling = RiskCeiling::Medium;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let mut loop_state = LoopState::new(2);

    let outcome = try_replan_avoidable_side_effect_free_freeform_clarify(
        &mut loop_state,
        Some(&route),
        &intent,
    )
    .expect("side-effect-free medium-risk freeform clarify should be recoverable");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(!loop_state.pending_user_input_required);
    assert!(loop_state.delivery_messages.is_empty());
    assert_eq!(loop_state.attempt_ledger_entries.len(), 1);
}

#[test]
fn high_risk_side_effect_free_clarify_does_not_replan() {
    let plan = plan_result_with_raw_and_steps(
        "{}",
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Need approval.",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_confirmation",
                "missing_slot": "user_input"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::Free);
    route.risk_ceiling = RiskCeiling::High;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    let mut loop_state = LoopState::new(2);
    loop_state.pending_user_boundary_present = true;

    assert!(try_replan_avoidable_side_effect_free_freeform_clarify(
        &mut loop_state,
        Some(&route),
        &intent,
    )
    .is_none());
    assert!(loop_state.attempt_ledger_entries.is_empty());
}

#[test]
fn locator_clarify_does_not_side_effect_free_freeform_replan() {
    let plan = plan_result_with_raw_and_steps(
        "{}",
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Which file should I read?",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_locator",
                "missing_slot": "locator",
                "locator_kind": "path"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.risk_ceiling = RiskCeiling::Low;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    let mut loop_state = LoopState::new(2);

    assert!(try_replan_avoidable_side_effect_free_freeform_clarify(
        &mut loop_state,
        Some(&route),
        &intent,
    )
    .is_none());
    assert!(loop_state.attempt_ledger_entries.is_empty());
}

#[test]
fn inconsistent_locator_clarify_without_route_boundary_replans_then_finishes_as_answer() {
    let plan = plan_result_with_raw_and_steps(
        "{}",
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Provide the target path.",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_locator",
                "missing_slot": "locator",
                "locator_kind": "path"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.risk_ceiling = RiskCeiling::Medium;
    route.needs_clarify = false;
    route.wants_file_delivery = false;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    let mut loop_state = LoopState::new(2);

    let first = try_recover_inconsistent_boundary_clarify(&mut loop_state, Some(&route), &intent)
        .expect("inconsistent boundary clarify should be recoverable");
    assert_eq!(
        first.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert!(!loop_state.pending_user_input_required);
    assert!(loop_state.delivery_messages.is_empty());
    assert!(loop_state.has_recoverable_failure_context);
    assert_eq!(loop_state.attempt_ledger_entries.len(), 1);

    loop_state.round_no = 2;
    let second = try_recover_inconsistent_boundary_clarify(&mut loop_state, Some(&route), &intent)
        .expect("repeated inconsistent boundary clarify should finish nonblocking");
    assert_eq!(
        second.stop_signal.as_deref(),
        Some("structured_respond_nonblocking_clarify_answer")
    );
    assert!(!loop_state.pending_user_input_required);
    assert_eq!(
        loop_state.delivery_messages,
        vec!["Provide the target path.".to_string()]
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.terminal_intent")
            .map(String::as_str),
        Some("answer")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.recovered_terminal_intent")
            .map(String::as_str),
        Some("clarify")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.clarify_reason_code")
            .map(String::as_str),
        Some("missing_locator")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.missing_slot")
            .map(String::as_str),
        Some("locator")
    );
}

#[test]
fn planner_locator_contract_does_not_recover_clarify_into_plan_file_read() {
    let plan = plan_result_with_raw_and_steps(
        "{}",
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "Which file should I read?",
                "terminal_intent": "clarify",
                "clarify_reason_code": "missing_locator",
                "missing_slot": "locator",
                "locator_kind": "path"
            }),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("structured intent");
    let mut route = route_result(OutputResponseShape::OneSentence);
    route.risk_ceiling = RiskCeiling::Low;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.delivery_required = false;
    route.output_contract.delivery_intent = OutputDeliveryIntent::None;
    let mut loop_state = LoopState::new(2);

    assert!(
        try_recover_inconsistent_boundary_clarify(&mut loop_state, Some(&route), &intent).is_none()
    );

    let outcome = apply_structured_respond_clarify_to_loop_state(&mut loop_state, &intent);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("structured_respond_clarify")
    );
    assert_eq!(
        loop_state.delivery_messages,
        vec!["Which file should I read?".to_string()]
    );
}

#[test]
fn decision_envelope_output_vars_do_not_expose_initial_gate_ref_as_field() {
    let route = route_result(OutputResponseShape::OneSentence);
    let plan = plan_result_with_raw_and_steps(
        r#"{"steps":[{"type":"respond","content":"ok"}]}"#,
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({"content": "ok"}),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let mut loop_state = LoopState::new(2);

    record_agent_loop_decision_envelope_output_vars(&mut loop_state, Some(&route), &plan);

    assert!(loop_state
        .output_vars
        .contains_key("agent_loop.decision_envelope"));
    let envelope: serde_json::Value = serde_json::from_str(
        loop_state
            .output_vars
            .get("agent_loop.decision_envelope")
            .expect("decision envelope"),
    )
    .expect("decision envelope json");
    assert!(envelope.get("initial_gate_ref").is_none());
    assert!(envelope.get("initial_hint_ref").is_none());
    assert!(envelope.get("fallback_gate_policy").is_none());
    assert!(!loop_state
        .output_vars
        .contains_key("agent_loop.initial_gate_ref"));
    assert!(!loop_state
        .output_vars
        .contains_key("agent_loop.decision_envelope.initial_gate_ref"));
}

#[test]
fn decision_envelope_output_vars_include_clarify_machine_fields_from_raw_plan() {
    let route = route_result(OutputResponseShape::OneSentence);
    let raw_plan = r#"{
        "steps": [{
            "type": "respond",
            "content": "Which file should I read?",
            "terminal_intent": "clarify",
            "clarify_reason_code": "missing_locator",
            "missing_slot": "locator",
            "locator_kind": "path"
        }]
    }"#;
    let plan = plan_result_with_raw_and_steps(
        raw_plan,
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({"content": "Which file should I read?"}),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let mut loop_state = LoopState::new(2);

    record_agent_loop_decision_envelope_output_vars(&mut loop_state, Some(&route), &plan);

    for (key, expected) in [
        ("agent_loop.terminal_intent", "clarify"),
        ("agent_loop.clarify_reason_code", "missing_locator"),
        ("agent_loop.missing_slot", "locator"),
        ("agent_loop.locator_kind", "path"),
        (
            "agent_loop.decision_envelope.clarify_reason_code",
            "missing_locator",
        ),
        ("agent_loop.decision_envelope.missing_slot", "locator"),
        ("agent_loop.decision_envelope.locator_kind", "path"),
    ] {
        assert_eq!(
            loop_state.output_vars.get(key).map(String::as_str),
            Some(expected),
            "missing {key}"
        );
    }
}

#[test]
fn decision_envelope_answer_clears_stale_clarify_machine_fields() {
    let route = route_result(OutputResponseShape::Free);
    let clarify_plan = plan_result_with_raw_and_steps(
        r#"{"steps":[{"type":"respond","content":"Need topic","terminal_intent":"clarify","clarify_reason_code":"missing_required_topic","missing_slot":"topic"}]}"#,
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({"content": "Need topic"}),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let answer_plan = plan_result_with_raw_and_steps(
        r#"{"steps":[{"type":"respond","content":"Draft answer"}]}"#,
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({"content": "Draft answer"}),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );
    let mut loop_state = LoopState::new(2);

    record_agent_loop_decision_envelope_output_vars(&mut loop_state, Some(&route), &clarify_plan);
    loop_state.pending_user_input_required = true;
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.terminal_intent")
            .map(String::as_str),
        Some("clarify")
    );
    assert!(loop_state
        .output_vars
        .contains_key("agent_loop.clarify_reason_code"));

    record_agent_loop_decision_envelope_output_vars(&mut loop_state, Some(&route), &answer_plan);

    assert!(!loop_state.pending_user_input_required);
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.terminal_intent")
            .map(String::as_str),
        Some("answer")
    );
    for key in [
        "agent_loop.clarify_reason_code",
        "agent_loop.missing_slot",
        "agent_loop.message_key",
        "agent_loop.field_path",
        "agent_loop.locator_kind",
        "agent_loop.decision_envelope.clarify_reason_code",
        "agent_loop.decision_envelope.missing_slot",
        "agent_loop.decision_envelope.message_key",
        "agent_loop.decision_envelope.field_path",
        "agent_loop.decision_envelope.locator_kind",
        "agent_loop.recovered_terminal_intent",
        "agent_loop.nonblocking_clarify_answer",
    ] {
        assert!(!loop_state.output_vars.contains_key(key), "stale {key}");
    }
}

#[test]
fn structured_respond_clarify_reads_raw_plan_when_normalized_step_loses_fields() {
    let raw_plan = r#"{
        "steps": [{
            "type": "respond",
            "content": "",
            "terminal_intent": "clarify",
            "clarify_reason_code": "missing_locator",
            "missing_slot": "locator",
            "field_path": "output_contract.locator_hint"
        }]
    }"#;
    let plan = plan_result_with_raw_and_steps(
        raw_plan,
        vec![crate::PlanStep {
            step_id: "step_1".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({"content": ""}),
            depends_on: Vec::new(),
            why: String::new(),
        }],
    );

    let intent = structured_respond_terminal_intent_from_plan(&plan).expect("raw intent");
    assert_eq!(intent.terminal_intent, "clarify");
    assert_eq!(
        intent.clarify_reason_code.as_deref(),
        Some("missing_locator")
    );
    assert_eq!(intent.missing_slot.as_deref(), Some("locator"));
    assert_eq!(
        intent.field_path.as_deref(),
        Some("output_contract.locator_hint")
    );
}

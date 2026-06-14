use serde_json::{json, Value};

pub(super) fn agent_loop_round_plan_decision_envelope_json(
    route: &crate::RouteResult,
    plan: &crate::PlanResult,
) -> Value {
    let actions = plan
        .steps
        .iter()
        .filter_map(crate::PlanStep::to_agent_action)
        .collect::<Vec<_>>();
    agent_loop_round_decision_envelope_json(route, &actions)
}

fn agent_loop_round_decision_envelope_json(
    route: &crate::RouteResult,
    actions: &[crate::AgentAction],
) -> Value {
    let output_contract_ref = output_contract_ref_for_route(route);
    agent_loop_decision_envelope_json(
        route,
        actions,
        &output_contract_ref,
        "planner_round_action",
        "planner_loop_runtime",
    )
}

pub(super) fn agent_loop_decision_envelope_json(
    route: &crate::RouteResult,
    actions: &[crate::AgentAction],
    output_contract_ref: &str,
    source: &str,
    semantic_authority: &str,
) -> Value {
    let contract = crate::TaskContract::from_route_result(route);
    let decision = agent_loop_decision_from_first_action(route, actions);
    let (validation_status, validation_reason_code) =
        agent_loop_decision_validation(route, actions, decision, &contract);
    json!({
        "schema_version": 1,
        "source": source,
        "initial_hint_ref": route.first_layer_decision().as_str(),
        "semantic_authority": semantic_authority,
        "fallback_gate_policy": "fallback_safety_check_only",
        "decision": decision,
        "reason_code": agent_loop_decision_reason_code(decision, actions),
        "validation_status": validation_status,
        "validation_reason_code": validation_reason_code,
        "confidence": null,
        "missing_slots": &contract.missing_parameters,
        "capability_ref": first_non_think_action_capability_ref(actions),
        "output_contract_ref": output_contract_ref,
        "required_evidence": &contract.required_evidence_fields,
        "risk_level": route.risk_ceiling.as_str(),
        "delivery_required": route.output_contract.delivery_required || route.wants_file_delivery,
        "language_rendering_policy": agent_loop_language_rendering_policy(decision),
    })
}

pub(super) fn output_contract_ref_for_route(route: &crate::RouteResult) -> String {
    let contract = &route.output_contract;
    format!(
        "semantic:{}|shape:{}|locator:{}|delivery:{}|content_evidence:{}",
        contract.semantic_kind.as_str(),
        contract.response_shape.as_str(),
        contract.locator_kind.as_str(),
        contract.delivery_intent.as_str(),
        contract.requires_content_evidence
    )
}

pub(super) fn first_non_think_action_decision(actions: &[crate::AgentAction]) -> &'static str {
    actions
        .iter()
        .find_map(|action| match action {
            crate::AgentAction::Think { .. } => None,
            crate::AgentAction::CallTool { .. } => Some("call_tool"),
            crate::AgentAction::CallSkill { .. } => Some("call_skill"),
            crate::AgentAction::CallCapability { .. } => Some("call_capability"),
            crate::AgentAction::SynthesizeAnswer { .. } => Some("synthesize_answer"),
            crate::AgentAction::Respond { .. } => Some("respond"),
        })
        .unwrap_or("no_action")
}

pub(super) fn first_non_think_action_capability_ref(
    actions: &[crate::AgentAction],
) -> Option<&str> {
    actions.iter().find_map(|action| match action {
        crate::AgentAction::Think { .. } => None,
        crate::AgentAction::CallCapability { capability, .. } => Some(capability.as_str()),
        crate::AgentAction::CallSkill { skill, .. } => Some(skill.as_str()),
        crate::AgentAction::CallTool { tool, .. } => Some(tool.as_str()),
        crate::AgentAction::SynthesizeAnswer { .. } => Some("synthesize_answer"),
        crate::AgentAction::Respond { .. } => Some("respond"),
    })
}

pub(super) fn first_layer_agent_decision_delta(
    first_layer: crate::FirstLayerDecision,
    agent_decision: &str,
) -> &'static str {
    use crate::FirstLayerDecision;
    let same_gate = match first_layer {
        FirstLayerDecision::Clarify => matches!(agent_decision, "respond"),
        FirstLayerDecision::DirectAnswer => {
            matches!(agent_decision, "respond" | "synthesize_answer")
        }
        FirstLayerDecision::PlannerExecute => {
            matches!(
                agent_decision,
                "call_tool" | "call_skill" | "call_capability"
            )
        }
    };
    if same_gate {
        "same_gate"
    } else if matches!(agent_decision, "think" | "no_action") {
        "not_comparable"
    } else {
        "different_gate"
    }
}

pub(super) fn agent_action_capability_delta(actions: &[crate::AgentAction]) -> &'static str {
    match actions
        .iter()
        .find(|action| !matches!(action, crate::AgentAction::Think { .. }))
    {
        Some(crate::AgentAction::CallCapability { .. }) => "agent_capability_ref",
        Some(crate::AgentAction::CallSkill { .. } | crate::AgentAction::CallTool { .. }) => {
            "agent_runtime_ref"
        }
        Some(crate::AgentAction::Respond { .. } | crate::AgentAction::SynthesizeAnswer { .. }) => {
            "no_capability_ref"
        }
        Some(crate::AgentAction::Think { .. }) | None => "not_comparable",
    }
}

fn agent_loop_decision_from_first_action(
    route: &crate::RouteResult,
    actions: &[crate::AgentAction],
) -> &'static str {
    match first_non_think_action_decision(actions) {
        "call_tool" | "call_skill" | "call_capability" => "call_capability",
        "synthesize_answer" => "synthesize_answer",
        "respond"
            if matches!(
                route.first_layer_decision(),
                crate::FirstLayerDecision::Clarify
            ) =>
        {
            "clarify"
        }
        "respond" => "respond",
        "no_action"
            if matches!(
                route.first_layer_decision(),
                crate::FirstLayerDecision::Clarify
            ) =>
        {
            "clarify"
        }
        "no_action" | "think" => "respond",
        _ => "respond",
    }
}

fn agent_loop_decision_reason_code(decision: &str, actions: &[crate::AgentAction]) -> &'static str {
    match decision {
        "call_capability" => "agent_loop_first_action_call_capability",
        "synthesize_answer" => "agent_loop_first_action_synthesize_answer",
        "clarify" => "agent_loop_first_action_clarify",
        "respond" if first_non_think_action_decision(actions) == "no_action" => {
            "agent_loop_first_action_no_action"
        }
        "respond" => "agent_loop_first_action_respond",
        _ => "agent_loop_first_action_unknown",
    }
}

fn agent_loop_language_rendering_policy(decision: &str) -> &'static str {
    match decision {
        "call_capability" => "defer_until_observation",
        "synthesize_answer" | "respond" | "clarify" => "finalizer_llm_i18n",
        _ => "finalizer_llm_i18n",
    }
}

fn agent_loop_decision_validation(
    route: &crate::RouteResult,
    actions: &[crate::AgentAction],
    decision: &str,
    contract: &crate::TaskContract,
) -> (&'static str, &'static str) {
    if decision == "respond"
        && route.output_contract.requires_content_evidence
        && first_non_think_action_decision(actions) == "respond"
    {
        return ("shadow_invalid", "respond_requires_evidence_observation");
    }
    if decision == "clarify" && contract.missing_parameters.is_empty() {
        return ("shadow_invalid", "clarify_missing_structured_slots");
    }
    ("valid", "agent_loop_decision_shadow_valid")
}

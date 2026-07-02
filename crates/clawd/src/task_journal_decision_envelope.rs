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
    let mut envelope = agent_loop_round_decision_envelope_json(route, &actions);
    if let Some(intent) = structured_respond_terminal_intent_from_plan(plan) {
        apply_structured_respond_terminal_intent(&mut envelope, intent);
    }
    envelope
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

#[derive(Debug, Clone)]
struct StructuredRespondTerminalIntent {
    terminal_intent: String,
    clarify_reason_code: Option<String>,
    missing_slot: Option<String>,
    message_key: Option<String>,
    field_path: Option<String>,
    locator_kind: Option<String>,
}

fn structured_respond_terminal_intent_from_plan(
    plan: &crate::PlanResult,
) -> Option<StructuredRespondTerminalIntent> {
    plan.steps
        .iter()
        .find_map(structured_respond_terminal_intent_from_plan_step)
        .or_else(|| {
            super::raw_plan_steps(&plan.raw_plan_text)
                .iter()
                .find_map(structured_respond_terminal_intent_from_raw_step)
        })
}

fn structured_respond_terminal_intent_from_plan_step(
    step: &crate::PlanStep,
) -> Option<StructuredRespondTerminalIntent> {
    (step.action_type == "respond")
        .then(|| structured_respond_terminal_intent_from_object(&step.args))?
}

fn structured_respond_terminal_intent_from_raw_step(
    step: &Value,
) -> Option<StructuredRespondTerminalIntent> {
    let raw_type = step
        .get("type")
        .or_else(|| step.get("action_type"))
        .or_else(|| step.get("action"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    (raw_type == "respond").then(|| structured_respond_terminal_intent_from_object(step))?
}

fn structured_respond_terminal_intent_from_object(
    value: &Value,
) -> Option<StructuredRespondTerminalIntent> {
    let terminal_intent = string_field(value, &["terminal_intent"])?.to_string();
    if !matches!(
        terminal_intent.as_str(),
        "answer" | "clarify" | "cannot_proceed" | "needs_confirmation" | "continue"
    ) {
        return None;
    }
    Some(StructuredRespondTerminalIntent {
        terminal_intent,
        clarify_reason_code: string_field(value, &["clarify_reason_code"]).map(str::to_string),
        missing_slot: string_field(value, &["missing_slot"]).map(str::to_string),
        message_key: string_field(value, &["message_key"]).map(str::to_string),
        field_path: string_field(value, &["field_path"]).map(str::to_string),
        locator_kind: string_field(value, &["locator_kind"]).map(str::to_string),
    })
}

fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn apply_structured_respond_terminal_intent(
    envelope: &mut Value,
    intent: StructuredRespondTerminalIntent,
) {
    let Some(obj) = envelope.as_object_mut() else {
        return;
    };
    let terminal_intent = intent.terminal_intent.as_str();
    if terminal_intent == "clarify" {
        obj.insert("decision".to_string(), json!("clarify"));
        obj.insert(
            "reason_code".to_string(),
            json!("agent_loop_respond_terminal_intent_clarify"),
        );
        obj.insert(
            "clarify_reason_code".to_string(),
            json!(intent
                .clarify_reason_code
                .as_deref()
                .unwrap_or("clarify_missing_structured_slots")),
        );
        if let Some(missing_slot) = intent.missing_slot.as_deref() {
            obj.insert("missing_slots".to_string(), json!([missing_slot]));
            obj.insert("missing_slot".to_string(), json!(missing_slot));
            obj.insert("validation_status".to_string(), json!("valid"));
            obj.insert(
                "validation_reason_code".to_string(),
                json!("agent_loop_decision_shadow_valid"),
            );
        } else {
            obj.insert("validation_status".to_string(), json!("shadow_invalid"));
            obj.insert(
                "validation_reason_code".to_string(),
                json!("clarify_missing_structured_slots"),
            );
        }
        obj.insert(
            "language_rendering_policy".to_string(),
            json!("finalizer_llm_i18n"),
        );
    }
    let control_intent = if terminal_intent == "clarify"
        && obj
            .get("validation_status")
            .and_then(Value::as_str)
            .is_some_and(|status| status == "shadow_invalid")
    {
        "recover"
    } else {
        structured_terminal_control_intent(terminal_intent)
    };
    let control_reason_code = if control_intent == "recover" && terminal_intent == "clarify" {
        "agent_loop_control_recover_invalid_clarify"
    } else {
        structured_terminal_control_reason_code(terminal_intent)
    };
    obj.insert("control_intent".to_string(), json!(control_intent));
    obj.insert(
        "control_reason_code".to_string(),
        json!(control_reason_code),
    );
    obj.insert("terminal_intent".to_string(), json!(intent.terminal_intent));
    if let Some(message_key) = intent.message_key {
        obj.insert("message_key".to_string(), json!(message_key));
    }
    if let Some(field_path) = intent.field_path {
        obj.insert("field_path".to_string(), json!(field_path));
    }
    if let Some(locator_kind) = intent.locator_kind {
        obj.insert("locator_kind".to_string(), json!(locator_kind));
    }
}

fn structured_terminal_control_intent(terminal_intent: &str) -> &'static str {
    match terminal_intent {
        "answer" => "answer",
        "clarify" => "clarify",
        "cannot_proceed" => "stop",
        "needs_confirmation" => "wait",
        "continue" => "recover",
        _ => "recover",
    }
}

fn structured_terminal_control_reason_code(terminal_intent: &str) -> &'static str {
    match terminal_intent {
        "answer" => "agent_loop_control_answer_terminal_intent",
        "clarify" => "agent_loop_control_clarify_terminal_intent",
        "cannot_proceed" => "agent_loop_control_stop_terminal_intent",
        "needs_confirmation" => "agent_loop_control_wait_terminal_intent",
        "continue" => "agent_loop_control_recover_terminal_intent",
        _ => "agent_loop_control_recover_unknown_terminal_intent",
    }
}

pub(super) fn agent_loop_decision_envelope_json(
    route: &crate::RouteResult,
    actions: &[crate::AgentAction],
    output_contract_ref: &str,
    source: &str,
    semantic_authority: &str,
) -> Value {
    let missing_parameters = crate::task_contract::missing_parameters_for_route(route);
    let required_evidence_fields = crate::task_contract::required_evidence_fields_for_route(route);
    let decision = agent_loop_decision_from_first_action(actions);
    let (validation_status, validation_reason_code) =
        agent_loop_decision_validation(route, actions, decision, &missing_parameters);
    let terminal_intent = agent_loop_terminal_intent(decision);
    let control_intent = agent_loop_control_intent(
        decision,
        terminal_intent,
        validation_status,
        validation_reason_code,
        actions,
    );
    let control_reason_code = agent_loop_control_reason_code(
        control_intent,
        terminal_intent,
        validation_status,
        validation_reason_code,
        actions,
    );
    let missing_slot = missing_parameters.first().map(String::as_str);
    let answer_shape = agent_loop_answer_shape(route);
    json!({
        "schema_version": 1,
        "source": source,
        "semantic_authority": semantic_authority,
        "decision": decision,
        "terminal_intent": terminal_intent,
        "control_intent": control_intent,
        "control_reason_code": control_reason_code,
        "reason_code": agent_loop_decision_reason_code(decision, actions),
        "clarify_reason_code": agent_loop_clarify_reason_code(
            decision,
            validation_reason_code,
        ),
        "validation_status": validation_status,
        "validation_reason_code": validation_reason_code,
        "confidence": null,
        "missing_slots": &missing_parameters,
        "missing_slot": missing_slot,
        "capability_ref": first_non_think_action_capability_ref(actions),
        "output_contract_ref": output_contract_ref,
        "required_evidence": &required_evidence_fields,
        "evidence_needed": &required_evidence_fields,
        "answer_shape": answer_shape,
        "risk_level": route.risk_ceiling.as_str(),
        "delivery_required": route.output_contract.delivery_required || route.wants_file_delivery,
        "language_rendering_policy": agent_loop_language_rendering_policy(decision),
    })
}

pub(super) fn output_contract_ref_for_route(route: &crate::RouteResult) -> String {
    let contract = route.effective_output_contract();
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

fn agent_loop_decision_from_first_action(actions: &[crate::AgentAction]) -> &'static str {
    match first_non_think_action_decision(actions) {
        "call_tool" | "call_skill" | "call_capability" => "call_capability",
        "synthesize_answer" => "synthesize_answer",
        "respond" => "respond",
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

fn agent_loop_terminal_intent(decision: &str) -> &'static str {
    match decision {
        "call_capability" => "continue",
        "clarify" => "clarify",
        "respond" | "synthesize_answer" => "answer",
        _ => "cannot_proceed",
    }
}

fn agent_loop_control_intent(
    decision: &str,
    terminal_intent: &str,
    validation_status: &'static str,
    _validation_reason_code: &'static str,
    actions: &[crate::AgentAction],
) -> &'static str {
    if validation_status == "shadow_invalid" {
        return "recover";
    }
    if first_non_think_action_decision(actions) == "no_action" {
        return "stop";
    }
    match terminal_intent {
        "answer" => "answer",
        "clarify" => "clarify",
        "needs_confirmation" => "wait",
        "cannot_proceed" => "stop",
        "continue" if decision == "call_capability" => "act",
        "continue" => "recover",
        _ => "recover",
    }
}

fn agent_loop_control_reason_code(
    control_intent: &str,
    terminal_intent: &str,
    validation_status: &'static str,
    validation_reason_code: &'static str,
    actions: &[crate::AgentAction],
) -> &'static str {
    if validation_status == "shadow_invalid" {
        return match validation_reason_code {
            "respond_requires_evidence_observation" => {
                "agent_loop_control_recover_missing_evidence"
            }
            "clarify_missing_structured_slots" => "agent_loop_control_recover_invalid_clarify",
            _ => "agent_loop_control_recover_shadow_invalid",
        };
    }
    if first_non_think_action_decision(actions) == "no_action" {
        return "agent_loop_control_stop_no_action";
    }
    match (control_intent, terminal_intent) {
        ("act", "continue") => "agent_loop_control_act_first_action",
        ("answer", "answer") => "agent_loop_control_answer_terminal_intent",
        ("clarify", "clarify") => "agent_loop_control_clarify_terminal_intent",
        ("wait", "needs_confirmation") => "agent_loop_control_wait_terminal_intent",
        ("stop", "cannot_proceed") => "agent_loop_control_stop_terminal_intent",
        ("recover", "continue") => "agent_loop_control_recover_terminal_intent",
        _ => "agent_loop_control_unknown",
    }
}

fn agent_loop_clarify_reason_code(
    decision: &str,
    validation_reason_code: &'static str,
) -> Option<&'static str> {
    (decision == "clarify").then_some(validation_reason_code)
}

fn agent_loop_answer_shape(route: &crate::RouteResult) -> String {
    crate::contract_matrix::final_answer_shape_for_route(route)
        .map(|shape| shape.as_str().to_string())
        .unwrap_or_else(|| route.output_contract.response_shape.as_str().to_string())
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
    missing_parameters: &[String],
) -> (&'static str, &'static str) {
    if decision == "respond"
        && route.output_contract.requires_content_evidence
        && first_non_think_action_decision(actions) == "respond"
    {
        return ("shadow_invalid", "respond_requires_evidence_observation");
    }
    if decision == "clarify" && missing_parameters.is_empty() {
        return ("shadow_invalid", "clarify_missing_structured_slots");
    }
    ("valid", "agent_loop_decision_shadow_valid")
}

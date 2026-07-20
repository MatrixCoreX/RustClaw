use crate::agent_engine::AgentRunContext;

pub(super) fn route_requires_content_evidence(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .map(|route| route.requires_content_evidence)
        .unwrap_or(false)
}

pub(super) fn preferred_route_clarify_question(
    _agent_run_context: Option<&AgentRunContext>,
) -> Option<&str> {
    None
}

pub(super) fn route_structured_clarify_context(
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    if !route.locator_hint.trim().is_empty() {
        return None;
    }
    let clarify_case = if route.delivery_required {
        "missing_file_locator"
    } else if route.requires_content_evidence {
        "missing_read_target"
    } else {
        "missing_user_input"
    };
    let final_answer_shape = crate::evidence_policy::final_answer_shape_for_output_contract(route);
    Some(
        [
            format!("clarify_case: {clarify_case}"),
            format!("locator_kind: {}", route.locator_kind.as_str()),
            format!("response_shape: {}", route.response_shape.as_str()),
            format!(
                "final_answer_shape: {}",
                final_answer_shape
                    .map(crate::evidence_policy::FinalAnswerShape::as_str)
                    .unwrap_or("none")
            ),
            format!(
                "requires_content_evidence: {}",
                route.requires_content_evidence
            ),
            format!("delivery_required: {}", route.delivery_required),
        ]
        .join("\n"),
    )
}

pub(super) fn route_output_contract_machine_json(
    route: &crate::IntentOutputContract,
) -> serde_json::Value {
    let final_answer_shape = crate::evidence_policy::final_answer_shape_for_output_contract(route);
    serde_json::json!({
        "response_shape": route.response_shape.as_str(),
        "final_answer_shape": final_answer_shape
            .map(crate::evidence_policy::FinalAnswerShape::as_str)
            .unwrap_or_else(|| route.response_shape.as_str()),
        "final_answer_shape_class": final_answer_shape.map(|shape| shape.class().as_str()),
        "locator_kind": route.locator_kind.as_str(),
        "delivery_required": route.delivery_required,
        "requires_content_evidence": route.requires_content_evidence
    })
}

pub(super) fn structured_json_values_from_output(output: &str) -> Vec<serde_json::Value> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return Vec::new();
    };
    let mut values = vec![value.clone()];
    if let Some(extra) = value.get("extra") {
        values.push(extra.clone());
    }
    values
}

pub(super) fn delivery_message_is_json_object(message: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(message.trim()),
        Ok(serde_json::Value::Object(_))
    )
}

pub(super) fn delivery_message_is_json_container(message: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(message.trim()),
        Ok(serde_json::Value::Object(_) | serde_json::Value::Array(_))
    )
}

use crate::agent_engine::AgentRunContext;

pub(super) fn route_requires_content_evidence(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.requires_content_evidence)
        .unwrap_or(false)
}

pub(super) fn preferred_route_clarify_question(
    agent_run_context: Option<&AgentRunContext>,
) -> Option<&str> {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .filter(|route| route.needs_clarify)
        .map(|route| route.clarify_question.trim())
        .filter(|question| !question.is_empty())
}

pub(super) fn route_structured_clarify_context(
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .filter(|route| route.needs_clarify)?;
    if !route.output_contract.locator_hint.trim().is_empty() {
        return None;
    }
    let clarify_case = route_clarify_reason_code(&route.route_reason).or_else(|| {
        if route.output_contract.delivery_required {
            Some("missing_file_locator")
        } else if route.output_contract.requires_content_evidence {
            Some("missing_read_target")
        } else {
            None
        }
    })?;
    let final_answer_shape = crate::evidence_policy::final_answer_shape_for_route(route);
    Some(
        [
            format!("clarify_case: {clarify_case}"),
            format!(
                "locator_kind: {}",
                route.output_contract.locator_kind.as_str()
            ),
            format!(
                "response_shape: {}",
                route.output_contract.response_shape.as_str()
            ),
            format!(
                "final_answer_shape: {}",
                final_answer_shape
                    .map(crate::evidence_policy::FinalAnswerShape::as_str)
                    .unwrap_or("none")
            ),
            format!(
                "requires_content_evidence: {}",
                route.output_contract.requires_content_evidence
            ),
            format!(
                "delivery_required: {}",
                route.output_contract.delivery_required
            ),
        ]
        .join("\n"),
    )
}

pub(super) fn route_output_contract_machine_json(route: &crate::RouteResult) -> serde_json::Value {
    let final_answer_shape = crate::evidence_policy::final_answer_shape_for_route(route);
    serde_json::json!({
        "response_shape": route.output_contract.response_shape.as_str(),
        "final_answer_shape": final_answer_shape.map(crate::evidence_policy::FinalAnswerShape::as_str),
        "final_answer_shape_class": final_answer_shape.map(|shape| shape.class().as_str()),
        "locator_kind": route.output_contract.locator_kind.as_str(),
        "delivery_required": route.output_contract.delivery_required,
        "requires_content_evidence": route.output_contract.requires_content_evidence
    })
}

pub(super) fn route_clarify_reason_code(route_reason: &str) -> Option<&'static str> {
    let code = crate::RouteReasonMarkers::new(route_reason).machine_value("clarify_reason_code")?;
    match code {
        "missing_count_target" => Some("missing_count_target"),
        "missing_delivery_locator" => Some("missing_delivery_locator"),
        "missing_file_locator" => Some("missing_file_locator"),
        "missing_locator" => Some("missing_locator"),
        "missing_service_target" => Some("missing_service_target"),
        "missing_search_locator" => Some("missing_search_locator"),
        "missing_read_target" => Some("missing_read_target"),
        "missing_target" => Some("missing_target"),
        _ => None,
    }
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

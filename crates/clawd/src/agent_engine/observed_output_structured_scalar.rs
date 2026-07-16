use super::*;

#[derive(Debug, Clone)]
pub(super) struct StructuredScalarObservation {
    pub(super) text: String,
    pub(super) source_key: String,
}

pub(super) fn structured_scalar_observation_from_extract_item(
    value: &serde_json::Value,
    parent: Option<&serde_json::Value>,
) -> Option<StructuredScalarObservation> {
    if !value
        .get("exists")
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
    {
        return None;
    }
    let raw_value = value.get("value").unwrap_or(&serde_json::Value::Null);
    if matches!(
        raw_value,
        serde_json::Value::Object(_) | serde_json::Value::Array(_)
    ) {
        return None;
    }
    value_structured_text(
        raw_value,
        value.get("value_text").and_then(|item| item.as_str()),
    )
    .map(|text| StructuredScalarObservation {
        text,
        source_key: structured_scalar_observation_source_key(value, parent),
    })
}

fn structured_scalar_observation_source_key(
    value: &serde_json::Value,
    parent: Option<&serde_json::Value>,
) -> String {
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .or_else(|| parent.and_then(|parent| parent.get("resolved_path")))
        .or_else(|| parent.and_then(|parent| parent.get("path")))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let field = value
        .get("resolved_field_path")
        .or_else(|| value.get("field_path"))
        .or_else(|| parent.and_then(|parent| parent.get("resolved_field_path")))
        .or_else(|| parent.and_then(|parent| parent.get("field_path")))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if path.is_empty() && field.is_empty() {
        String::new()
    } else {
        format!(
            "{}\n{}",
            path.to_ascii_lowercase(),
            field.to_ascii_lowercase()
        )
    }
}

fn structured_scalar_observation_from_step(
    step: &crate::executor::StepExecutionResult,
) -> Option<StructuredScalarObservation> {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "config_basic") {
        return None;
    }
    let body = step.output.as_deref()?.trim();
    if body.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    structured_scalar_observation_from_value(&value).or_else(|| {
        value
            .get("extra")
            .and_then(structured_scalar_observation_from_value)
    })
}

pub(super) fn structured_scalar_observation_from_value(
    value: &serde_json::Value,
) -> Option<StructuredScalarObservation> {
    match value.get("action").and_then(|item| item.as_str()) {
        Some("extract_field" | "read_field") => {
            structured_scalar_observation_from_extract_item(value, None)
        }
        Some("extract_fields" | "read_fields") => {
            let results = value.get("results")?.as_array()?;
            let mut scalar_results = results.iter().filter_map(|item| {
                structured_scalar_observation_from_extract_item(item, Some(value))
            });
            let scalar = scalar_results.next()?;
            if scalar_results.next().is_some() {
                return None;
            }
            Some(scalar)
        }
        _ => None,
    }
}

fn recent_structured_scalar_observations(
    loop_state: &LoopState,
    limit: usize,
) -> Vec<StructuredScalarObservation> {
    let mut recent = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter_map(structured_scalar_observation_from_step)
        .take(limit.max(1))
        .collect::<Vec<_>>();
    recent.reverse();
    recent
}

pub(crate) fn recent_structured_scalar_observation_count(loop_state: &LoopState) -> usize {
    recent_structured_scalar_observations(loop_state, 2).len()
}

#[cfg(test)]
pub(crate) fn latest_structured_scalar_observation_text(loop_state: &LoopState) -> Option<String> {
    recent_structured_scalar_observations(loop_state, 1)
        .into_iter()
        .next()
        .map(|observation| observation.text)
}

pub(super) fn multiple_structured_scalar_observations_need_synthesis(
    route: Option<&crate::IntentOutputContract>,
    loop_state: &LoopState,
) -> bool {
    let observations = recent_structured_scalar_observations(loop_state, 2);
    if observations.len() < 2 {
        return false;
    }
    if !observations[0].source_key.is_empty()
        && observations[0].source_key == observations[1].source_key
    {
        return false;
    }
    !route.is_some_and(|route| {
        super::output_route_policy::route_contract_marker_is_any(
            route,
            &[
                crate::OutputSemanticKind::RecentScalarEqualityCheck,
                crate::OutputSemanticKind::QuantityComparison,
            ],
        )
    })
}

pub(crate) fn structured_scalar_equality_direct_answer(
    _state: Option<&AppState>,
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    _agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::RecentScalarEqualityCheck,
    ) || route.delivery_required
    {
        return None;
    }
    if let Some(answer) = latest_compare_paths_scalar_equality_answer(loop_state) {
        return Some(answer);
    }
    let observations = recent_structured_scalar_observations(loop_state, 2);
    if observations.len() < 2 {
        return None;
    }
    let left = observations[0].text.trim();
    let right = observations[1].text.trim();
    if left.is_empty() || right.is_empty() {
        return None;
    }
    if !observations[0].source_key.is_empty()
        && observations[0].source_key == observations[1].source_key
    {
        return None;
    }
    let same = left == right;
    Some(format!("{left} {} {right}", if same { "==" } else { "!=" }))
}

fn latest_compare_paths_scalar_equality_answer(loop_state: &LoopState) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "fs_basic" | "system_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(compare_paths_scalar_equality_answer_from_body)
}

fn compare_paths_scalar_equality_answer_from_body(body: &str) -> Option<String> {
    let raw = serde_json::from_str::<serde_json::Value>(body.trim()).ok()?;
    let value = raw
        .get("extra")
        .filter(|extra| {
            extra.is_object()
                && extra
                    .get("action")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
        })
        .unwrap_or(&raw);
    if value.get("action").and_then(serde_json::Value::as_str) != Some("compare_paths") {
        return None;
    }
    let field_value = value.get("field_value").filter(|value| value.is_object());
    let same_path = field_value
        .and_then(|item| item.get("same_path"))
        .or_else(|| {
            value
                .get("comparison")
                .and_then(|item| item.get("same_path"))
        })
        .and_then(serde_json::Value::as_bool)?;
    let left_exists = field_value
        .and_then(|item| item.get("left_exists"))
        .or_else(|| value.get("left").and_then(|item| item.get("exists")))
        .and_then(serde_json::Value::as_bool)?;
    let right_exists = field_value
        .and_then(|item| item.get("right_exists"))
        .or_else(|| value.get("right").and_then(|item| item.get("exists")))
        .and_then(serde_json::Value::as_bool)?;
    Some(format!(
        "same_path={same_path}\nleft_exists={left_exists}\nright_exists={right_exists}"
    ))
}

pub(super) fn route_needs_structured_scalar_pair_synthesis(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(|route| {
            recent_structured_scalar_observation_count(loop_state) > 1
                && super::output_route_policy::route_contract_marker_is_any(
                    route,
                    &[
                        crate::OutputSemanticKind::RecentScalarEqualityCheck,
                        crate::OutputSemanticKind::QuantityComparison,
                    ],
                )
        })
}

use super::*;

pub(super) fn evidence_policy_scalar_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let Some(shape) =
        crate::evidence_policy::final_answer_shape_for_output_contract(&route.output_contract)
    else {
        return false;
    };
    if shape.class() != crate::evidence_policy::FinalAnswerShapeClass::ScalarValue {
        return false;
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
        && (!scalar_answer_is_strict(candidate_answer)
            || route.output_contract.response_shape != crate::OutputResponseShape::Scalar)
    {
        return count_summary_answer_is_grounded_in_successful_observation(
            route,
            journal,
            candidate_answer,
            route.output_contract.response_shape != crate::OutputResponseShape::Scalar,
        );
    }
    scalar_answer_is_strict(candidate_answer)
        && scalar_answer_value_is_grounded_in_successful_observation(
            route,
            journal,
            candidate_answer,
        )
}

pub(super) fn count_summary_answer_is_grounded_in_successful_observation(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
    allow_single_observed_scalar: bool,
) -> bool {
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    if allow_single_observed_scalar && candidate.lines().count() > 1 {
        return false;
    }
    let mut observed_values = observed_scalar_values_from_evidence_map_for_route(route, journal);
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation_for_route(route, step) {
            continue;
        }
        if !step_can_supply_strict_evidence_for_route(route, step) {
            continue;
        }
        if let Some(output) = step.output_excerpt.as_deref() {
            observed_values.extend(observed_scalar_values_from_output(output));
        }
    }
    observed_values
        .iter()
        .filter(|observed| scalar_token_occurs_in_text(candidate, observed))
        .collect::<BTreeSet<_>>()
        .len()
        >= if allow_single_observed_scalar { 1 } else { 2 }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedScalarValue {
    pub(super) source_key: String,
    pub(super) text: String,
}

#[cfg(test)]
pub(super) fn recent_structured_scalar_values_from_journal(
    journal: &crate::task_journal::TaskJournal,
    limit: usize,
) -> Vec<ObservedScalarValue> {
    let mut recent = journal
        .step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .filter_map(observed_scalar_value_from_step_output)
        .take(limit.max(1))
        .collect::<Vec<_>>();
    recent.reverse();
    recent
}

#[cfg(test)]
pub(super) fn observed_scalar_value_from_step_output(output: &str) -> Option<ObservedScalarValue> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    observed_scalar_value_from_json(&value)
        .or_else(|| value.get("extra").and_then(observed_scalar_value_from_json))
}

#[cfg(test)]
pub(super) fn observed_scalar_value_from_json(
    value: &serde_json::Value,
) -> Option<ObservedScalarValue> {
    match value.get("action").and_then(|item| item.as_str()) {
        Some("extract_field" | "read_field") => {
            observed_scalar_value_from_extract_item(value, None)
        }
        Some("extract_fields" | "read_fields") => {
            let results = value.get("results")?.as_array()?;
            if results.len() != 1 {
                return None;
            }
            observed_scalar_value_from_extract_item(results.first()?, Some(value))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(super) fn observed_scalar_value_from_extract_item(
    item: &serde_json::Value,
    parent: Option<&serde_json::Value>,
) -> Option<ObservedScalarValue> {
    if item.get("exists").and_then(|value| value.as_bool()) == Some(false) {
        return None;
    }
    let text = item
        .get("value_text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            item.get("value")
                .or_else(|| item.get("field_value"))
                .and_then(observed_scalar_json_value_text)
        })?;
    let path = item
        .get("resolved_path")
        .or_else(|| item.get("path"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            parent.and_then(|parent| {
                parent
                    .get("resolved_path")
                    .or_else(|| parent.get("path"))
                    .and_then(|value| value.as_str())
            })
        })
        .unwrap_or("")
        .trim();
    let field = item
        .get("resolved_field_path")
        .or_else(|| item.get("field_path"))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    let source_key = format!("{path}#{field}");
    Some(ObservedScalarValue { source_key, text })
}

#[cfg(test)]
pub(super) fn observed_scalar_json_value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

pub(super) fn observed_scalar_values_from_output(output: &str) -> BTreeSet<String> {
    let mut values = BTreeSet::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
        collect_scalar_values_from_json(&value, &mut values);
    } else {
        for line in output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            if line.parse::<i64>().is_ok() {
                values.insert(line.to_string());
            }
        }
    }
    values
}

pub(super) fn collect_scalar_values_from_json(
    value: &serde_json::Value,
    values: &mut BTreeSet<String>,
) {
    match value {
        serde_json::Value::Number(value) => {
            values.insert(value.to_string());
        }
        serde_json::Value::Bool(value) => {
            values.insert(value.to_string());
        }
        serde_json::Value::String(value) => {
            let value = value.trim();
            if value.parse::<i64>().is_ok() {
                values.insert(value.to_string());
            }
        }
        serde_json::Value::Array(items) => {
            values.insert(items.len().to_string());
            for item in items {
                collect_scalar_values_from_json(item, values);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values() {
                collect_scalar_values_from_json(value, values);
            }
        }
        serde_json::Value::Null => {}
    }
}

pub(super) fn scalar_token_occurs_in_text(text: &str, scalar: &str) -> bool {
    let scalar = scalar.trim();
    !scalar.is_empty()
        && text
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .any(|token| token == scalar)
}

pub(super) fn scalar_answer_is_strict(candidate_answer: &str) -> bool {
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() || candidate_answer.lines().count() > 1 {
        return false;
    }
    let lower = candidate_answer.to_ascii_lowercase();
    if lower.contains(" is ") || lower.contains("：") || lower.contains(':') {
        return false;
    }
    if candidate_answer.ends_with('.') || candidate_answer.ends_with('。') {
        return false;
    }
    true
}

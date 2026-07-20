use super::*;

pub(super) fn final_answer_preserves_service_control_status_summary(
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    let contract = route_result.clone();
    if contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Strict
        )
        || !route_allows_model_language_delivery(route_result, contract.response_shape)
    {
        return false;
    }
    let status_values = service_control_status_observed_values(journal);
    if status_values.is_empty() {
        return false;
    }
    std::iter::once(answer_text)
        .chain(answer_messages.iter().map(String::as_str))
        .any(|candidate| candidate_matches_service_control_status(candidate, &status_values))
}

fn service_control_status_observed_values(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    let mut values = Vec::new();
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || step.skill != "service_control"
        {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(payload) = service_control_payload_from_output(output) else {
            continue;
        };
        for key in ["post_state", "pre_state", "summary"] {
            if let Some(value) = payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                push_service_control_status_value(&mut values, value);
            }
        }
    }
    values.sort();
    values.dedup();
    values
}

fn service_control_payload_from_output(output: &str) -> Option<serde_json::Value> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    if json_value_has_service_control_status_shape(&value) {
        return Some(value);
    }
    value
        .get("extra")
        .filter(|extra| json_value_has_service_control_status_shape(extra))
        .cloned()
}

fn json_value_has_service_control_status_shape(value: &serde_json::Value) -> bool {
    (value.get("post_state").is_some()
        || value.get("pre_state").is_some()
        || value.get("summary").is_some())
        && (value.get("service_name").is_some() || value.get("target").is_some())
}

fn push_service_control_status_value(values: &mut Vec<String>, value: &str) {
    push_status_value_if_publishable(values, value);
    if let Some((_, tail)) = value.rsplit_once('=') {
        push_status_value_if_publishable(values, tail.trim());
    }
    if let Some((_, tail)) = value.rsplit_once(':') {
        push_status_value_if_publishable(values, tail.trim());
    }
}

fn push_status_value_if_publishable(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.len() < 3
        || value.eq_ignore_ascii_case("ok")
        || value.contains('\n')
        || !value.chars().any(|ch| ch.is_ascii_alphanumeric())
    {
        return;
    }
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn candidate_matches_service_control_status(candidate: &str, status_values: &[String]) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
        || text_is_json_object_or_array(candidate)
        || text_looks_like_raw_command_snapshot(candidate)
        || text_is_machine_kv_only(candidate)
    {
        return false;
    }
    status_values
        .iter()
        .any(|value| candidate_has_observed_status_value(candidate, value))
}

fn candidate_has_observed_status_value(candidate: &str, observed: &str) -> bool {
    if candidate.contains(observed) {
        return true;
    }
    if !observed.is_ascii() {
        return false;
    }
    candidate
        .to_ascii_lowercase()
        .contains(observed.to_ascii_lowercase().as_str())
}

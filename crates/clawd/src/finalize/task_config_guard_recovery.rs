use serde_json::{json, Value};

fn answer_verifier_requests_config_candidates(journal: &crate::task_journal::TaskJournal) -> bool {
    journal
        .answer_verifier_summary
        .as_ref()
        .filter(|summary| summary.high_confidence_retry_gap())
        .is_some_and(|summary| {
            summary
                .missing_evidence_fields
                .iter()
                .any(|field| field.trim() == "candidates")
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigGuardObservation {
    path: Option<String>,
    risk_count: Option<u64>,
    valid: Option<bool>,
    candidates: Vec<String>,
}

fn config_guard_route_allows_failure_recovery(route_result: &crate::RouteResult) -> bool {
    if route_result.output_contract.delivery_required {
        return false;
    }
    crate::machine_capability_ref::route_has_capability_action(
        route_result,
        &["config"],
        &["guard", "risk", "validate"],
    ) || matches!(
        route_result.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::ConfigRiskAssessment
            | crate::OutputSemanticKind::ConfigValidation
    )
}

fn config_guard_action_matches(action: &str) -> bool {
    matches!(
        action.trim(),
        "guard_config" | "guard_rustclaw_config" | "validate_guard_config"
    )
}

fn string_array_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn config_guard_observation_from_flat_value(value: &Value) -> Option<ConfigGuardObservation> {
    let action_matches = value
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(config_guard_action_matches);
    let candidates = {
        let primary = string_array_field(value, "candidates");
        if primary.is_empty() {
            string_array_field(value, "risks")
        } else {
            primary
        }
    };
    let risk_count = value
        .get("risk_count")
        .or_else(|| value.get("count"))
        .and_then(Value::as_u64);
    if !action_matches && candidates.is_empty() && risk_count.is_none() {
        return None;
    }
    let path = value
        .get("path")
        .or_else(|| value.get("resolved_path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string);
    Some(ConfigGuardObservation {
        path,
        risk_count,
        valid: value.get("valid").and_then(Value::as_bool),
        candidates,
    })
}

fn config_guard_observation_from_value(value: &Value) -> Option<ConfigGuardObservation> {
    config_guard_observation_from_flat_value(value).or_else(|| {
        value
            .get("extra")
            .and_then(config_guard_observation_from_value)
    })
}

fn machine_json_string_array_after_key(output: &str, key: &str) -> Option<Vec<String>> {
    let marker = format!("\"{key}\":[");
    let start = output.find(&marker)? + marker.len() - 1;
    let rest = output.get(start..)?;
    let end = rest.find(']')?;
    serde_json::from_str::<Vec<String>>(rest.get(..=end)?).ok()
}

fn machine_json_u64_after_key(output: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = output.find(&marker)? + marker.len();
    let digits = output
        .get(start..)?
        .chars()
        .skip_while(|ch| ch.is_ascii_whitespace())
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse::<u64>().ok())?
}

fn machine_json_string_after_key(output: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = output.find(&marker)? + marker.len();
    let rest = output.get(start..)?;
    let end = rest.find('"')?;
    rest.get(..end)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn config_guard_observation_from_truncated_output(output: &str) -> Option<ConfigGuardObservation> {
    let candidates = machine_json_string_array_after_key(output, "candidates")
        .or_else(|| machine_json_string_array_after_key(output, "risks"))
        .unwrap_or_default()
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    let risk_count = machine_json_u64_after_key(output, "risk_count")
        .or_else(|| machine_json_u64_after_key(output, "count"));
    if candidates.is_empty() && risk_count.is_none() {
        return None;
    }
    Some(ConfigGuardObservation {
        path: machine_json_string_after_key(output, "path")
            .or_else(|| machine_json_string_after_key(output, "resolved_path")),
        risk_count,
        valid: None,
        candidates,
    })
}

fn config_guard_observation_from_step_output(output: &str) -> Option<ConfigGuardObservation> {
    serde_json::from_str::<Value>(output.trim())
        .ok()
        .and_then(|value| config_guard_observation_from_value(&value))
        .or_else(|| config_guard_observation_from_truncated_output(output))
}

fn config_guard_observation_answer(observation: ConfigGuardObservation) -> String {
    let risk_count = observation
        .risk_count
        .unwrap_or(observation.candidates.len() as u64);
    let mut payload = json!({
        "message_key": "clawd.msg.config_edit.guard",
        "reason_code": if risk_count == 0 { "config_edit_guard_no_risk" } else { "config_edit_guard_risk_found" },
        "risk_count": risk_count,
        "count": risk_count,
        "candidates": observation.candidates,
    });
    if let Some(path) = observation.path {
        payload["path"] = json!(path);
    }
    if let Some(valid) = observation.valid {
        payload["valid"] = json!(valid);
    }
    payload.to_string()
}

pub(super) fn deterministic_config_guard_candidates_recovery(
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if !config_guard_route_allows_failure_recovery(route_result)
        || !answer_verifier_requests_config_candidates(journal)
    {
        return None;
    }
    journal.step_results.iter().rev().find_map(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || !matches!(
                step.skill.as_str(),
                "config_basic" | "config_edit" | "config_guard"
            )
        {
            return None;
        }
        step.output_excerpt
            .as_deref()
            .and_then(config_guard_observation_from_step_output)
            .map(config_guard_observation_answer)
    })
}

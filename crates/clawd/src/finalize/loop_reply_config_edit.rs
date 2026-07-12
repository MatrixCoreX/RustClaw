use std::path::Path;

#[cfg(test)]
use crate::agent_engine::LoopState;
use crate::AppState;

#[cfg(test)]
use super::single_publishable_delivery_message;
use super::{
    deterministic_observed_execution_status_summary, execution_summary_value_to_string,
    prefer_english_for_user_text,
};

#[derive(Debug)]
struct ConfigEditObservedOutput {
    index: usize,
    value: serde_json::Value,
}

fn config_edit_output_action(value: &serde_json::Value) -> Option<&str> {
    value
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn config_edit_observable_action(action: &str) -> bool {
    matches!(
        action,
        "plan_config_change"
            | "apply_config_change"
            | "validate_config"
            | "guard_config"
            | "extract_field"
            | "extract_fields"
            | "read_field"
            | "read_fields"
            | "read_back"
            | "restart_if_requested"
    )
}

fn config_edit_observed_value_from_output(output: &str) -> Option<serde_json::Value> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    config_edit_observed_value_candidate(&value)
}

fn config_edit_observed_value_candidate(value: &serde_json::Value) -> Option<serde_json::Value> {
    if config_edit_output_action(value).is_some_and(config_edit_observable_action) {
        return Some(value.clone());
    }
    value
        .get("extra")
        .and_then(config_edit_observed_value_candidate)
}

fn step_may_contain_config_edit_observation(step: &crate::executor::StepExecutionResult) -> bool {
    matches!(
        step.skill.as_str(),
        "config_edit" | "config_basic" | "config_guard"
    ) || step.skill.starts_with("config.")
        || step.skill.starts_with("config_edit.")
        || step.skill.starts_with("config_basic.")
}

fn config_edit_observed_outputs(
    loop_state: &crate::agent_engine::LoopState,
) -> Vec<ConfigEditObservedOutput> {
    let latest_config_edit_step = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| step.skill == "config_edit");
    if latest_config_edit_step.is_some_and(|step| !step.is_ok()) {
        return Vec::new();
    }
    loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            if !step.is_ok() {
                return None;
            }
            let value = config_edit_observed_value_from_output(step.output.as_deref()?)?;
            if !step_may_contain_config_edit_observation(step)
                && !config_edit_value_has_target(&value)
            {
                return None;
            }
            Some(ConfigEditObservedOutput { index, value })
        })
        .collect()
}

fn config_edit_value_has_target(value: &serde_json::Value) -> bool {
    value.get("path").is_some()
        || value.get("resolved_path").is_some()
        || value.get("field_path").is_some()
        || value.get("resolved_field_path").is_some()
}

fn config_edit_machine_payload(value: &serde_json::Value) -> bool {
    let message_key = config_edit_string_field(value, "message_key");
    let reason_code = config_edit_string_field(value, "reason_code");
    if message_key == Some("clawd.msg.config_edit.preview_read_guard")
        || reason_code == Some("config_edit_preview_read_guard")
    {
        return false;
    }
    let config_message = message_key.is_some_and(|key| key.starts_with("clawd.msg.config_edit."));
    let config_reason = reason_code.is_some_and(|code| code.starts_with("config_edit_"));
    (config_message || config_reason)
        && (value.get("field_path").is_some()
            || value.get("path").is_some()
            || value.get("risk_count").is_some()
            || value.get("would_change").is_some()
            || value.get("would_write").is_some()
            || value.get("applied").is_some())
}

pub(super) fn direct_config_edit_terminal_machine_payload_answer(
    loop_state: &crate::agent_engine::LoopState,
) -> Option<String> {
    if let Some(answer) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .and_then(config_edit_machine_payload_text)
    {
        return Some(answer);
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok() && matches!(step.skill.as_str(), "synthesize_answer" | "respond")
        })
        .filter_map(|step| step.output.as_deref())
        .find_map(config_edit_machine_payload_text)
}

pub(super) fn config_edit_machine_payload_text(output: &str) -> Option<String> {
    let output = output.trim();
    if output.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
    config_edit_machine_payload_value_text(&value)
        .or_else(|| config_edit_machine_payload(&value).then(|| output.to_string()))
}

fn config_edit_machine_payload_value_text(value: &serde_json::Value) -> Option<String> {
    if config_edit_machine_payload(value) {
        return Some(value.to_string());
    }
    if let Some(text) = value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        if let Some(answer) = config_edit_machine_payload_text(text) {
            return Some(answer);
        }
    }
    value
        .get("extra")
        .and_then(config_edit_machine_payload_value_text)
}

fn config_edit_string_field<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn config_edit_path_label(value: &serde_json::Value) -> &str {
    config_edit_string_field(value, "path")
        .or_else(|| config_edit_string_field(value, "resolved_path"))
        .unwrap_or("config")
}

fn config_edit_field_label(value: &serde_json::Value) -> &str {
    config_edit_string_field(value, "field_path").unwrap_or("field")
}

fn config_edit_value_label(value: &serde_json::Value, primary_key: &str) -> Option<String> {
    config_edit_string_field(value, "value_text")
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get(primary_key)
                .map(execution_summary_value_to_string)
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
        })
}

fn config_edit_output_matches_field(
    value: &serde_json::Value,
    field_path: &str,
    path: &str,
) -> bool {
    config_edit_string_field(value, "field_path") == Some(field_path)
        && config_edit_string_field(value, "path")
            .or_else(|| config_edit_string_field(value, "resolved_path"))
            .is_none_or(|candidate| candidate == path)
}

fn config_edit_summary() -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: 1,
        ..Default::default()
    }
}

fn direct_config_edit_apply_answer(
    outputs: &[ConfigEditObservedOutput],
    _prefer_english: bool,
) -> Option<String> {
    let applied = outputs.iter().rev().find(|item| {
        config_edit_output_action(&item.value) == Some("apply_config_change")
            && item.value.get("applied").and_then(|value| value.as_bool()) == Some(true)
    })?;
    let field_path = config_edit_field_label(&applied.value);
    let path = config_edit_path_label(&applied.value);
    let read_back = outputs.iter().rev().find(|item| {
        item.index > applied.index
            && config_edit_output_action(&item.value) == Some("read_back")
            && item.value.get("exists").and_then(|value| value.as_bool()) == Some(true)
            && config_edit_output_matches_field(&item.value, field_path, path)
    });
    let value_label = read_back
        .and_then(|item| config_edit_value_label(&item.value, "value"))
        .or_else(|| config_edit_value_label(&applied.value, "new_value"));
    let validation_after_apply = outputs.iter().rev().find(|item| {
        item.index > applied.index
            && config_edit_output_action(&item.value) == Some("validate_config")
            && config_edit_path_label(&item.value) == path
    });
    let validation_passed = validation_after_apply
        .and_then(|item| item.value.get("valid").and_then(|value| value.as_bool()))
        .or_else(|| {
            applied
                .value
                .get("validated")
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(false);
    let guard = outputs.iter().rev().find(|item| {
        item.index > applied.index
            && config_edit_output_action(&item.value) == Some("guard_config")
            && config_edit_path_label(&item.value) == path
    });
    let risk_count = guard.and_then(|item| item.value.get("risk_count").and_then(|v| v.as_u64()));

    let mut payload = serde_json::json!({
        "message_key": "clawd.msg.config_edit.applied",
        "reason_code": "config_edit_applied",
        "field_path": field_path,
        "path": path,
        "validation_passed": validation_passed,
    });
    if let Some(value) = value_label {
        payload["value"] = serde_json::json!(value);
    }
    if let Some(risk_count) = risk_count {
        payload["risk_count"] = serde_json::json!(risk_count);
    }
    Some(payload.to_string())
}

fn direct_config_edit_plan_answer(
    outputs: &[ConfigEditObservedOutput],
    _prefer_english: bool,
) -> Option<String> {
    let planned = outputs.iter().rev().find(|item| {
        config_edit_output_action(&item.value) == Some("plan_config_change")
            && !outputs.iter().any(|candidate| {
                candidate.index > item.index
                    && config_edit_output_action(&candidate.value) == Some("apply_config_change")
            })
    })?;
    let field_path = config_edit_field_label(&planned.value);
    let path = config_edit_path_label(&planned.value);
    let value = config_edit_value_label(&planned.value, "new_value");
    let would_change = planned
        .value
        .get("would_change")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let mut payload = serde_json::json!({
        "message_key": "clawd.msg.config_edit.planned",
        "reason_code": "config_edit_planned",
        "field_path": field_path,
        "path": path,
        "would_change": would_change,
        "applied": false,
    });
    if let Some(value) = value {
        payload["value"] = serde_json::json!(value);
    }
    if let Some(guard) = outputs.iter().rev().find(|item| {
        item.index > planned.index
            && config_edit_output_action(&item.value) == Some("guard_config")
            && config_edit_path_label(&item.value) == path
    }) {
        let risk_count = guard
            .value
            .get("risk_count")
            .and_then(|value| value.as_u64())
            .unwrap_or_else(|| config_edit_risk_labels(&guard.value).len() as u64);
        payload["risk_count"] = serde_json::json!(risk_count);
        payload["count"] = serde_json::json!(risk_count);
        payload["risks"] = serde_json::json!(config_edit_risk_labels(&guard.value));
        payload["candidates"] = serde_json::json!(config_edit_candidate_labels(&guard.value));
    }
    Some(payload.to_string())
}

fn direct_config_edit_validate_answer(
    outputs: &[ConfigEditObservedOutput],
    _prefer_english: bool,
) -> Option<String> {
    let validation = outputs
        .iter()
        .rev()
        .find(|item| config_edit_output_action(&item.value) == Some("validate_config"))?;
    let path = config_edit_path_label(&validation.value);
    let valid = validation.value.get("valid")?.as_bool()?;
    let mut payload = serde_json::json!({
        "message_key": "clawd.msg.config_edit.validation",
        "reason_code": if valid { "config_edit_validation_passed" } else { "config_edit_validation_failed" },
        "path": path,
        "valid": valid,
    });
    if !valid {
        payload["error_text"] =
            serde_json::json!(
                config_edit_string_field(&validation.value, "error_text").unwrap_or("invalid")
            );
    }
    Some(payload.to_string())
}

fn config_edit_risk_labels(value: &serde_json::Value) -> Vec<String> {
    value
        .get("risks")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn config_edit_candidate_labels(value: &serde_json::Value) -> Vec<String> {
    value
        .get("candidates")
        .or_else(|| value.get("risks"))
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn path_is_agent_guard_config(path: &str) -> bool {
    let components = Path::new(path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    matches!(components.as_slice(), [.., "configs", "agent_guard.toml"])
}

fn config_field_result<'a>(
    value: &'a serde_json::Value,
    field_path: &str,
) -> Option<&'a serde_json::Value> {
    value
        .get("results")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .find(|item| {
            item.get("field_path")
                .or_else(|| item.get("resolved_field_path"))
                .and_then(serde_json::Value::as_str)
                == Some(field_path)
        })
}

fn config_field_exists(value: &serde_json::Value, field_path: &str) -> bool {
    config_field_result(value, field_path)
        .and_then(|item| item.get("exists"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn config_field_array_count(value: &serde_json::Value, field_path: &str) -> usize {
    config_field_result(value, field_path)
        .and_then(|item| item.get("value"))
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn agent_hook_policy_surface_answer(
    outputs: &[ConfigEditObservedOutput],
    _prefer_english: bool,
) -> Option<String> {
    let observation = outputs.iter().rev().find(|item| {
        matches!(
            config_edit_output_action(&item.value),
            Some("extract_fields" | "read_fields")
        ) && config_edit_string_field(&item.value, "path")
            .or_else(|| config_edit_string_field(&item.value, "resolved_path"))
            .is_some_and(path_is_agent_guard_config)
    })?;

    let blocked_action_refs = "agent.hooks.blocked_action_refs";
    let blocked_tools = "agent.hooks.blocked_tools";
    let require_confirmation = "agent.hooks.require_confirmation_action_refs";
    let background_wait = "agent.hooks.background_wait_action_refs";
    let deny_supported = config_field_exists(&observation.value, blocked_action_refs)
        || config_field_exists(&observation.value, blocked_tools);
    let payload = serde_json::json!({
        "message_key": "clawd.msg.agent_hooks.pre_tool_use_policy_surface",
        "reason_code": "agent_hooks_pre_tool_use_policy_surface",
        "owner_layer": "agent_hooks",
        "stage": "pre_tool_use",
        "path": "configs/agent_guard.toml",
        "read_only": true,
        "would_mutate": false,
        "decision_tokens": ["allow", "deny", "require_confirmation", "background_wait"],
        "field_paths": [
            blocked_action_refs,
            blocked_tools,
            require_confirmation,
            background_wait
        ],
        "decisions": {
            "allow": {
                "supported": true,
                "source": "default_allow"
            },
            "deny": {
                "supported": deny_supported,
                "fields": [blocked_action_refs, blocked_tools],
                "configured_ref_count": config_field_array_count(&observation.value, blocked_action_refs)
                    + config_field_array_count(&observation.value, blocked_tools)
            },
            "require_confirmation": {
                "supported": config_field_exists(&observation.value, require_confirmation),
                "field": require_confirmation,
                "configured_ref_count": config_field_array_count(&observation.value, require_confirmation)
            },
            "background_wait": {
                "supported": config_field_exists(&observation.value, background_wait),
                "field": background_wait,
                "configured_ref_count": config_field_array_count(&observation.value, background_wait)
            }
        }
    });
    Some(payload.to_string())
}

fn direct_config_edit_guard_answer(
    outputs: &[ConfigEditObservedOutput],
    _prefer_english: bool,
) -> Option<String> {
    let guard = outputs
        .iter()
        .rev()
        .find(|item| config_edit_output_action(&item.value) == Some("guard_config"))?;
    let path = config_edit_path_label(&guard.value);
    let risk_count = guard
        .value
        .get("risk_count")
        .and_then(|value| value.as_u64())
        .unwrap_or_else(|| config_edit_risk_labels(&guard.value).len() as u64);
    let risks = config_edit_risk_labels(&guard.value);
    let candidates = config_edit_candidate_labels(&guard.value);
    Some(
        serde_json::json!({
            "message_key": "clawd.msg.config_edit.guard",
            "reason_code": if risk_count == 0 { "config_edit_guard_no_risk" } else { "config_edit_guard_risk_found" },
            "path": path,
            "risk_count": risk_count,
            "count": risk_count,
            "candidates": candidates,
            "risks": risks,
        })
        .to_string(),
    )
}

fn direct_config_edit_read_guard_answer(
    outputs: &[ConfigEditObservedOutput],
    _prefer_english: bool,
) -> Option<String> {
    let read = outputs.iter().rev().find(|item| {
        matches!(
            config_edit_output_action(&item.value),
            Some("extract_field" | "read_field")
        ) && item
            .value
            .get("exists")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    })?;
    let field_path = config_edit_field_label(&read.value);
    let path = config_edit_path_label(&read.value);
    let value = config_edit_value_label(&read.value, "value")?;
    let guard = outputs.iter().rev().find(|item| {
        item.index > read.index
            && config_edit_output_action(&item.value) == Some("guard_config")
            && config_edit_path_label(&item.value) == path
    });
    let risk_count = guard
        .and_then(|item| {
            item.value
                .get("risk_count")
                .and_then(|value| value.as_u64())
        })
        .unwrap_or(0);
    let risks = guard
        .map(|item| config_edit_risk_labels(&item.value))
        .unwrap_or_default();
    let candidates = guard
        .map(|item| config_edit_candidate_labels(&item.value))
        .unwrap_or_default();
    Some(
        serde_json::json!({
            "message_key": "clawd.msg.config_edit.read_guard",
            "reason_code": "config_edit_read_guard",
            "path": path,
            "field_path": field_path,
            "current_value": value,
            "risk_count": risk_count,
            "count": risk_count,
            "risks": risks,
            "candidates": candidates,
        })
        .to_string(),
    )
}

fn direct_config_edit_read_back_answer(
    outputs: &[ConfigEditObservedOutput],
    _prefer_english: bool,
) -> Option<String> {
    let read_back = outputs
        .iter()
        .rev()
        .find(|item| config_edit_output_action(&item.value) == Some("read_back"))?;
    let field_path = config_edit_field_label(&read_back.value);
    let path = config_edit_path_label(&read_back.value);
    let exists = read_back
        .value
        .get("exists")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let mut payload = serde_json::json!({
        "message_key": "clawd.msg.config_edit.read_back",
        "reason_code": if exists { "config_edit_read_back_found" } else { "config_edit_read_back_missing" },
        "field_path": field_path,
        "path": path,
        "exists": exists,
    });
    if exists {
        payload["value"] = serde_json::json!(
            config_edit_value_label(&read_back.value, "value").unwrap_or_default()
        );
    }
    Some(payload.to_string())
}

pub(crate) fn direct_config_edit_observed_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if let Some(answer) = direct_config_edit_terminal_machine_payload_answer(loop_state) {
        return Some((answer, config_edit_summary()));
    }
    let outputs = config_edit_observed_outputs(loop_state);
    if outputs.is_empty() {
        return None;
    }
    let request_language = crate::language_policy::request_language_hint(user_text);
    let prefer_english = request_language == "en"
        || (request_language == "config_default" && prefer_english_for_user_text(state, user_text));
    let answer = direct_config_edit_apply_answer(&outputs, prefer_english)
        .or_else(|| direct_config_edit_plan_answer(&outputs, prefer_english))
        .or_else(|| agent_hook_policy_surface_answer(&outputs, prefer_english))
        .or_else(|| direct_config_edit_read_guard_answer(&outputs, prefer_english))
        .or_else(|| direct_config_edit_guard_answer(&outputs, prefer_english))
        .or_else(|| direct_config_edit_validate_answer(&outputs, prefer_english))
        .or_else(|| direct_config_edit_read_back_answer(&outputs, prefer_english))?;
    Some((answer, config_edit_summary()))
}

fn direct_config_guard_observed_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let outputs = config_edit_observed_outputs(loop_state);
    if outputs.is_empty() {
        return None;
    }
    let prefer_english = prefer_english_for_user_text(state, user_text);
    direct_config_edit_guard_answer(&outputs, prefer_english).map(|answer| {
        (
            answer,
            deterministic_observed_execution_status_summary(loop_state),
        )
    })
}

#[derive(Debug)]
struct RustClawConfigFieldObservation {
    path: String,
    field_path: String,
    exists: bool,
    value: serde_json::Value,
    value_text: Option<String>,
}

fn path_is_rustclaw_main_config(path: &str) -> bool {
    let components = Path::new(path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    matches!(components.as_slice(), [.., "configs", "config.toml"])
}

fn rustclaw_config_path_label(path: &str) -> String {
    if path_is_rustclaw_main_config(path) {
        "configs/config.toml".to_string()
    } else {
        path.to_string()
    }
}

fn config_output_path(value: &serde_json::Value) -> Option<String> {
    value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

fn observed_config_field_path(value: &serde_json::Value) -> Option<String> {
    value
        .get("resolved_field_path")
        .or_else(|| value.get("field_path"))
        .or_else(|| value.get("field"))
        .or_else(|| value.get("key"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
}

fn rustclaw_config_field_observation_from_value(
    path: &str,
    value: &serde_json::Value,
) -> Option<RustClawConfigFieldObservation> {
    let field_path = observed_config_field_path(value)?;
    let field_value = value
        .get("value")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let exists = value
        .get("exists")
        .and_then(|value| value.as_bool())
        .unwrap_or(!field_value.is_null());
    let value_text = value
        .get("value_text")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    Some(RustClawConfigFieldObservation {
        path: path.to_string(),
        field_path,
        exists,
        value: field_value,
        value_text,
    })
}

fn rustclaw_config_field_observations_from_output(
    value: &serde_json::Value,
) -> Vec<RustClawConfigFieldObservation> {
    let Some(action) = value.get("action").and_then(|value| value.as_str()) else {
        return Vec::new();
    };
    if !matches!(
        action,
        "extract_field" | "extract_fields" | "read_field" | "read_fields"
    ) {
        return Vec::new();
    }
    let Some(path) = config_output_path(value).filter(|path| path_is_rustclaw_main_config(path))
    else {
        return Vec::new();
    };
    if let Some(results) = value.get("results").and_then(|value| value.as_array()) {
        return results
            .iter()
            .filter_map(|item| rustclaw_config_field_observation_from_value(&path, item))
            .collect();
    }
    rustclaw_config_field_observation_from_value(&path, value)
        .into_iter()
        .collect()
}

fn rustclaw_config_field_observations(
    loop_state: &crate::agent_engine::LoopState,
) -> Vec<RustClawConfigFieldObservation> {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok() && matches!(step.skill.as_str(), "config_basic" | "system_basic")
        })
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<serde_json::Value>(output.trim()).ok())
        .flat_map(|value| rustclaw_config_field_observations_from_output(&value))
        .collect()
}

fn observed_field_value_text(observation: &RustClawConfigFieldObservation) -> Option<String> {
    observation.value_text.clone().or_else(|| {
        if observation.value.is_string() {
            observation.value.as_str().map(ToString::to_string)
        } else if observation.value.is_null() {
            None
        } else {
            Some(execution_summary_value_to_string(&observation.value))
        }
    })
}

fn observed_field_is_true(observation: &RustClawConfigFieldObservation) -> bool {
    observation.value.as_bool() == Some(true)
        || observed_field_value_text(observation)
            .is_some_and(|text| text.trim().eq_ignore_ascii_case("true") || text.trim() == "1")
}

fn observed_field_i64(observation: &RustClawConfigFieldObservation) -> Option<i64> {
    observation.value.as_i64().or_else(|| {
        observed_field_value_text(observation)?
            .trim()
            .parse::<i64>()
            .ok()
    })
}

fn observed_tools_allow_contains_wildcard(observation: &RustClawConfigFieldObservation) -> bool {
    if observation
        .value
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("*")))
    {
        return true;
    }
    observed_field_value_text(observation).is_some_and(|text| {
        text.split(',')
            .map(|part| part.trim().trim_matches('"').trim_matches('\''))
            .any(|part| part == "*" || part == "[*]")
    })
}

fn observed_server_listen_is_public(observation: &RustClawConfigFieldObservation) -> bool {
    observed_field_value_text(observation)
        .map(|text| text.trim().trim_matches('"').to_string())
        .is_some_and(|text| text == "0.0.0.0" || text.starts_with("0.0.0.0:"))
}

fn quoted_string_label(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
}

fn rustclaw_config_known_risk_field(field_path: &str) -> bool {
    [
        "tools.allow",
        "tools.allow_sudo",
        "tools.allow_path_outside_workspace",
        "telegram.sendfile.full_access",
        "server.listen",
        "self_extension.enabled",
        "worker.task_timeout_seconds",
    ]
    .iter()
    .any(|candidate| field_path.eq_ignore_ascii_case(candidate))
}

fn rustclaw_config_risk_label(observation: &RustClawConfigFieldObservation) -> Option<String> {
    if !observation.exists {
        return None;
    }
    let field_path = observation.field_path.trim();
    if field_path.eq_ignore_ascii_case("tools.allow") {
        return observed_tools_allow_contains_wildcard(observation)
            .then(|| "tools.allow=[\"*\"]".to_string());
    }
    if field_path.eq_ignore_ascii_case("tools.allow_sudo") {
        return observed_field_is_true(observation).then(|| "tools.allow_sudo=true".to_string());
    }
    if field_path.eq_ignore_ascii_case("tools.allow_path_outside_workspace") {
        return observed_field_is_true(observation)
            .then(|| "tools.allow_path_outside_workspace=true".to_string());
    }
    if field_path.eq_ignore_ascii_case("telegram.sendfile.full_access") {
        return observed_field_is_true(observation)
            .then(|| "telegram.sendfile.full_access=true".to_string());
    }
    if field_path.eq_ignore_ascii_case("server.listen") {
        return observed_server_listen_is_public(observation).then(|| {
            let value = observed_field_value_text(observation).unwrap_or_default();
            format!(
                "server.listen={}",
                quoted_string_label(value.trim().trim_matches('"'))
            )
        });
    }
    if field_path.eq_ignore_ascii_case("self_extension.enabled") {
        return observed_field_is_true(observation)
            .then(|| "self_extension.enabled=true".to_string());
    }
    if field_path.eq_ignore_ascii_case("worker.task_timeout_seconds") {
        let value = observed_field_i64(observation)?;
        return (value > 3600).then(|| format!("worker.task_timeout_seconds={value}"));
    }
    None
}

fn direct_rustclaw_config_field_risk_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let observations = rustclaw_config_field_observations(loop_state);
    let mut known_fields = Vec::new();
    let mut risks = Vec::new();
    for observation in &observations {
        if !rustclaw_config_known_risk_field(&observation.field_path) {
            continue;
        }
        if !known_fields
            .iter()
            .any(|field: &String| field.eq_ignore_ascii_case(&observation.field_path))
        {
            known_fields.push(observation.field_path.clone());
        }
        if let Some(label) = rustclaw_config_risk_label(observation) {
            if !risks.iter().any(|existing| existing == &label) {
                risks.push(label);
            }
        }
    }
    if known_fields.len() < 2 {
        return None;
    }
    let path = observations
        .iter()
        .find(|observation| rustclaw_config_known_risk_field(&observation.field_path))
        .map(|observation| rustclaw_config_path_label(&observation.path))
        .unwrap_or_else(|| "configs/config.toml".to_string());
    let _ = (state, user_text);
    let answer = serde_json::json!({
        "message_key": "clawd.msg.config_risk.summary",
        "reason_code": if risks.is_empty() { "config_risk_none_found" } else { "config_risk_found" },
        "path": path,
        "risk_count": risks.len(),
        "risks": risks,
    })
    .to_string();
    Some((
        answer,
        deterministic_observed_execution_status_summary(loop_state),
    ))
}

pub(super) fn direct_rustclaw_config_risk_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    direct_config_guard_observed_answer(state, user_text, loop_state)
        .or_else(|| direct_rustclaw_config_field_risk_answer(state, user_text, loop_state))
}

#[cfg(test)]
pub(super) fn delivery_matches_config_guard_answer(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    let outputs = config_edit_observed_outputs(loop_state);
    if outputs.is_empty() {
        return false;
    }
    [true, false].into_iter().any(|prefer_english| {
        direct_config_edit_guard_answer(&outputs, prefer_english)
            .is_some_and(|answer| answer.trim() == delivery_text)
    })
}

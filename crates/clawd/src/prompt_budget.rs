use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};
use claw_core::model_turn::ModelToolDefinition;

#[derive(Debug, Clone, Copy)]
pub(crate) struct PromptSection<'a> {
    pub(crate) name: &'static str,
    pub(crate) text: &'a str,
    pub(crate) cacheability: &'static str,
    pub(crate) provenance: &'static str,
    pub(crate) omission_reason: Option<&'static str>,
}

pub(crate) fn prompt_section_budget_report(
    prompt_label: &str,
    sections: &[PromptSection<'_>],
) -> Value {
    let section_reports = sections
        .iter()
        .map(|section| {
            let estimate = crate::token_estimator::estimate_generic_tokens(section.text);
            let included = section.omission_reason.is_none() && !section.text.is_empty();
            json!({
                "name": section.name,
                "included": included,
                "byte_count": estimate.byte_count,
                "char_count": estimate.char_count,
                "token_estimate": estimate.provider_tokens,
                "token_safety_estimate": estimate.safety_tokens,
                "token_estimator": estimate.estimator.as_str(),
                "cacheability": section.cacheability,
                "provenance": section.provenance,
                "omission_reason": section.omission_reason,
            })
        })
        .collect::<Vec<_>>();
    let total = |key: &str| {
        section_reports
            .iter()
            .filter_map(|section| section.get(key).and_then(Value::as_u64))
            .sum::<u64>()
    };
    json!({
        "schema_version": 1,
        "prompt_label": prompt_label,
        "section_count": section_reports.len(),
        "included_section_count": section_reports
            .iter()
            .filter(|section| section.get("included").and_then(Value::as_bool) == Some(true))
            .count(),
        "byte_count": total("byte_count"),
        "char_count": total("char_count"),
        "token_estimate": total("token_estimate"),
        "token_safety_estimate": total("token_safety_estimate"),
        "sections": section_reports,
    })
}

pub(crate) fn publish_prompt_section_budget_report(
    state: &AppState,
    task: &ClaimedTask,
    prompt_label: &str,
    sections: &[PromptSection<'_>],
) {
    let report = prompt_section_budget_report(prompt_label, sections);
    if let Err(error) = crate::task_event_transport::publish_claimed_event(
        state,
        task,
        "prompt_section_budget",
        report,
    ) {
        tracing::warn!(
            task_id = task.task_id,
            prompt_label,
            error = %error,
            "prompt_section_budget_event_publish_failed"
        );
    }
}

pub(crate) fn model_tool_surface_budget_report(
    prompt_label: &str,
    tools: &[ModelToolDefinition],
    callable_capability_count: usize,
    eager_group_count: usize,
    selected_group_count: usize,
    disclosure_mode: &str,
) -> Value {
    let serialized = serde_json::to_string(tools).unwrap_or_default();
    let estimate = crate::token_estimator::estimate_generic_tokens(&serialized);
    let tool_reports = tools
        .iter()
        .map(|tool| {
            let schema = serde_json::to_string(&tool.input_schema).unwrap_or_default();
            let schema_estimate = crate::token_estimator::estimate_generic_tokens(&schema);
            let capability_enum_count = tool
                .input_schema
                .pointer("/properties/capability/enum")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            json!({
                "name": tool.name,
                "description_char_count": tool.description.chars().count(),
                "schema_byte_count": schema_estimate.byte_count,
                "schema_char_count": schema_estimate.char_count,
                "schema_token_estimate": schema_estimate.provider_tokens,
                "capability_enum_count": capability_enum_count,
                "strict": tool.strict,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema_version": 1,
        "prompt_label": prompt_label,
        "disclosure_mode": disclosure_mode,
        "tool_count": tools.len(),
        "callable_capability_count": callable_capability_count,
        "eager_group_count": eager_group_count,
        "selected_group_count": selected_group_count,
        "serialized_byte_count": estimate.byte_count,
        "serialized_char_count": estimate.char_count,
        "serialized_token_estimate": estimate.provider_tokens,
        "serialized_token_safety_estimate": estimate.safety_tokens,
        "token_estimator": estimate.estimator.as_str(),
        "tools": tool_reports,
    })
}

pub(crate) fn publish_model_tool_surface_budget_report(
    state: &AppState,
    task: &ClaimedTask,
    prompt_label: &str,
    tools: &[ModelToolDefinition],
    callable_capability_count: usize,
    eager_group_count: usize,
    selected_group_count: usize,
    disclosure_mode: &str,
) {
    let report = model_tool_surface_budget_report(
        prompt_label,
        tools,
        callable_capability_count,
        eager_group_count,
        selected_group_count,
        disclosure_mode,
    );
    if let Err(error) = crate::task_event_transport::publish_claimed_event(
        state,
        task,
        "model_tool_surface_budget",
        report,
    ) {
        tracing::warn!(
            task_id = task.task_id,
            prompt_label,
            error = %error,
            "model_tool_surface_budget_event_publish_failed"
        );
    }
}

#[cfg(test)]
#[path = "prompt_budget_tests.rs"]
mod tests;

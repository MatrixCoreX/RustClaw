use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

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

#[cfg(test)]
#[path = "prompt_budget_tests.rs"]
mod tests;

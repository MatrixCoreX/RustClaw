use serde_json::Value;

pub(super) fn verified_terminal_answer_after_verifier_pass(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    let verifier = journal.answer_verifier_summary.as_ref()?;
    if !verifier.pass || verifier.high_confidence_retry_gap() {
        return None;
    }
    journal
        .step_results
        .iter()
        .rev()
        .find_map(verified_terminal_answer_from_step)
}

fn verified_terminal_answer_from_step(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> Option<String> {
    if !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        || step.status != crate::executor::StepExecutionStatus::Ok
    {
        return None;
    }
    let answer = step.output_excerpt.as_deref()?.trim();
    if answer.is_empty()
        || serde_json::from_str::<Value>(answer).is_ok()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
    {
        return None;
    }
    Some(answer.to_string())
}

pub(super) fn deterministic_structured_evidence_table_recovery(
    route: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    current_failure_reply: bool,
) -> Option<String> {
    if route.output_contract.delivery_required || route.wants_file_delivery {
        return None;
    }
    let verifier = journal.answer_verifier_summary.as_ref()?;
    let verifier_requested_structured_rewrite = verifier.high_confidence_retry_gap()
        && verifier
            .missing_evidence_fields
            .iter()
            .any(|field| field == "output_format")
        && verifier
            .missing_evidence_fields
            .iter()
            .any(|field| field == "field_value");
    let stale_failure_after_verifier_pass = current_failure_reply
        && verifier.pass
        && !verifier.high_confidence_retry_gap()
        && latest_terminal_answer_is_machine_json(journal);
    if !verifier_requested_structured_rewrite && !stale_failure_after_verifier_pass {
        return None;
    }
    if !crate::task_journal::evidence_coverage_for_route(route, journal).is_complete() {
        return None;
    }
    structured_evidence_table_from_journal(journal)
}

fn latest_terminal_answer_is_machine_json(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.step_results.iter().rev().any(|step| {
        matches!(step.skill.as_str(), "respond" | "synthesize_answer")
            && step.status == crate::executor::StepExecutionStatus::Ok
            && step
                .output_excerpt
                .as_deref()
                .map(str::trim)
                .filter(|answer| !answer.is_empty())
                .is_some_and(|answer| serde_json::from_str::<Value>(answer).is_ok())
    })
}

fn structured_evidence_table_from_journal(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    let rows = structured_field_value_rows(journal);
    if rows.len() < 2 {
        return None;
    }
    let mut out = String::from("| field | value |\n| --- | --- |\n");
    for (field, value) in rows {
        out.push_str("| ");
        out.push_str(&escape_markdown_table_cell(&field));
        out.push_str(" | ");
        out.push_str(&escape_markdown_table_cell(&value));
        out.push_str(" |\n");
    }
    Some(out.trim_end().to_string())
}

fn structured_field_value_rows(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<(String, String)> {
    let mut rows = std::collections::BTreeMap::<String, String>::new();
    for step in journal.step_results.iter().rev().filter(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
    }) {
        let Some(evidence) = crate::task_journal::observed_evidence_for_step_trace(step) else {
            continue;
        };
        collect_structured_field_value_rows(&step.skill, &evidence, &mut rows);
    }
    rows.into_iter().collect()
}

fn collect_structured_field_value_rows(
    skill: &str,
    evidence: &Value,
    rows: &mut std::collections::BTreeMap<String, String>,
) {
    let Some(items) = evidence.get("items").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let Some(field) = item.get("field").and_then(Value::as_str) else {
            continue;
        };
        let Some(label) = projected_field_value_label(skill, field) else {
            continue;
        };
        let Some(value) = observed_evidence_item_value(item) else {
            continue;
        };
        rows.entry(label).or_insert(value);
    }
}

fn projected_field_value_label(skill: &str, field: &str) -> Option<String> {
    let suffix = field.strip_prefix("extra.field_value.")?;
    if suffix.is_empty() || suffix == "action" {
        return None;
    }
    let domain = skill.strip_suffix("_basic").unwrap_or(skill);
    Some(format!("{domain}.{suffix}"))
}

fn observed_evidence_item_value(item: &Value) -> Option<String> {
    if let Some(values) = item.get("sample_values").and_then(Value::as_array) {
        let rendered = values
            .iter()
            .filter_map(observed_evidence_scalar_to_string)
            .collect::<Vec<_>>();
        if !rendered.is_empty() {
            return Some(rendered.join(", "));
        }
    }
    item.get("excerpt")
        .and_then(observed_evidence_scalar_to_string)
        .map(|value| value.replace(['\n', '\r'], " ").trim().to_string())
        .filter(|value| !value.is_empty())
}

fn observed_evidence_scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
    .filter(|value| !value.is_empty())
}

fn escape_markdown_table_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\n', '\r'], " ")
        .trim()
        .to_string()
}

#[cfg(test)]
#[path = "task_structured_evidence_table_tests.rs"]
mod tests;

use super::*;

pub(in crate::agent_engine::loop_control) fn try_recover_structured_evidence_table_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract.delivery_required || route.wants_file_delivery {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    let Some(verifier) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    if !verifier.high_confidence_retry_gap()
        || !verifier
            .missing_evidence_fields
            .iter()
            .any(|field| field == "output_format")
        || !verifier
            .missing_evidence_fields
            .iter()
            .any(|field| field == "field_value")
    {
        return false;
    }
    if !crate::task_journal::evidence_coverage_for_route(route, journal).is_complete() {
        return false;
    }
    let Some(answer) = structured_evidence_table_from_journal(journal) else {
        return false;
    };
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer.clone();
    reply.messages = vec![answer];
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_structured_evidence_table");
    true
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

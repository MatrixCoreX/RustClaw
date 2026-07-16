use super::*;

pub(in crate::agent_engine::loop_control) fn try_recover_structured_evidence_table_answer_verifier_gap(
    route_result: Option<&crate::answer_verifier::AnswerContract>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract.delivery_required {
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
        || (!verifier
            .missing_evidence_fields
            .iter()
            .any(|field| field == "field_value")
            && structured_evidence_table_from_journal(journal).is_none())
    {
        return false;
    }
    if !crate::task_journal::evidence_coverage_for_output_contract(
        &route.effective_output_contract(),
        journal,
    )
    .is_complete()
    {
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
        collect_structured_rows_from_step_output(
            &step.skill,
            step.output_excerpt.as_deref(),
            &mut rows,
        );
        let Some(evidence) = crate::task_journal::observed_evidence_for_step_trace(step) else {
            continue;
        };
        collect_structured_field_value_rows(&step.skill, &evidence, &mut rows);
    }
    rows.into_iter().collect()
}

fn collect_structured_rows_from_step_output(
    skill: &str,
    output_excerpt: Option<&str>,
    rows: &mut std::collections::BTreeMap<String, String>,
) {
    let Some(output_excerpt) = output_excerpt
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return;
    };
    let Ok(Value::Object(output)) = serde_json::from_str::<Value>(output_excerpt) else {
        return;
    };
    let Some(extra) = output.get("extra").and_then(Value::as_object) else {
        return;
    };
    let domain = structured_skill_domain(skill);
    if let Some(field_value) = extra.get("field_value").and_then(Value::as_object) {
        collect_machine_object_rows(domain, field_value, rows);
    }
    for key in [
        "entries",
        "content_excerpt",
        "tables",
        "schema_version",
        "user_version",
        "table_count",
        "count",
        "total",
    ] {
        if let Some(value) = extra.get(key).and_then(machine_value_to_cell) {
            rows.entry(format!("{domain}.{key}")).or_insert(value);
        }
    }
    if let Some(result) = extra.get("result").and_then(Value::as_object) {
        collect_result_rows(domain, result, rows);
    }
}

fn collect_machine_object_rows(
    domain: &str,
    object: &serde_json::Map<String, Value>,
    rows: &mut std::collections::BTreeMap<String, String>,
) {
    for (key, value) in object {
        if key == "action" {
            continue;
        }
        if let Some(value) = machine_value_to_cell(value) {
            rows.entry(format!("{domain}.{key}")).or_insert(value);
        }
    }
}

fn collect_result_rows(
    domain: &str,
    result: &serde_json::Map<String, Value>,
    rows: &mut std::collections::BTreeMap<String, String>,
) {
    let Some(result_rows) = result.get("rows").and_then(Value::as_array) else {
        return;
    };
    for result_row in result_rows {
        let Some(result_row) = result_row.as_object() else {
            continue;
        };
        collect_machine_object_rows(domain, result_row, rows);
    }
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
    let domain = structured_skill_domain(skill);
    Some(format!("{domain}.{suffix}"))
}

fn structured_skill_domain(skill: &str) -> &str {
    skill.strip_suffix("_basic").unwrap_or(skill)
}

fn machine_value_to_cell(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Array(values) => {
            let rendered = values
                .iter()
                .filter_map(machine_value_to_cell)
                .collect::<Vec<_>>();
            (!rendered.is_empty()).then(|| rendered.join(", "))
        }
        Value::Object(object) => {
            if let Some(name) = object
                .get("name")
                .or_else(|| object.get("path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(name.to_string());
            }
            let rendered = object
                .iter()
                .filter_map(|(key, value)| {
                    machine_value_to_cell(value).map(|value| format!("{key}={value}"))
                })
                .collect::<Vec<_>>();
            (!rendered.is_empty()).then(|| rendered.join(", "))
        }
        Value::Null => None,
    }
    .filter(|value| !value.is_empty())
}

fn observed_evidence_item_value(item: &Value) -> Option<String> {
    if let Some(values) = item.get("sample_values").and_then(Value::as_array) {
        let rendered = values
            .iter()
            .filter_map(machine_value_to_cell)
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

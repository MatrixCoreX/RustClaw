use serde_json::Value;
use std::path::Path;

pub(super) fn deterministic_tree_summary_rows_failure_recovery(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if journal.final_stop_signal.as_deref() != Some("synthesize_answer_failed") {
        return None;
    }
    journal
        .step_results
        .iter()
        .rev()
        .filter_map(|step| {
            if step.status != crate::executor::StepExecutionStatus::Ok
                || !step.skill.eq_ignore_ascii_case("system_basic")
            {
                return None;
            }
            let output = step.output_excerpt.as_deref()?;
            let value = serde_json::from_str::<Value>(output.trim()).ok()?;
            tree_summary_rows_answer_from_value(&value)
        })
        .max_by_key(|answer| answer.lines().count())
}

fn tree_summary_rows_answer_from_value(value: &Value) -> Option<String> {
    if value.get("action").and_then(Value::as_str) == Some("tree_summary") {
        return tree_summary_rows_machine_lines(value);
    }
    if let Some(extra) = value.get("extra") {
        if extra.get("action").and_then(Value::as_str) == Some("tree_summary") {
            return tree_summary_rows_machine_lines(extra);
        }
    }
    None
}

fn tree_summary_rows_machine_lines(value: &Value) -> Option<String> {
    let rows = value
        .get("summary_rows")
        .or_else(|| value.get("candidates"))
        .or_else(|| value.get("results"))
        .and_then(Value::as_array)?;
    let lines = rows
        .iter()
        .filter_map(tree_summary_row_machine_line)
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn tree_summary_row_machine_line(row: &Value) -> Option<String> {
    if row.get("kind").and_then(Value::as_str) != Some("dir") {
        return None;
    }
    let name = row
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            row.get("path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .and_then(|path| Path::new(path).file_name().and_then(|name| name.to_str()))
        })?;
    let file_count = row.get("file_count").and_then(Value::as_u64)?;
    let truncated = row
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(format!(
        "name={name} file_count={file_count} truncated={truncated}"
    ))
}

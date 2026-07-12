use serde_json::{json, Value};
use std::path::Path;

use super::{TaskJournal, TaskJournalStepTrace};

pub(super) fn validation_result_json(journal: &TaskJournal) -> Value {
    let code_context = journal_has_code_or_test_artifact_step(journal);
    let signals = journal
        .step_results
        .iter()
        .filter_map(|step| validation_signal_from_step(step, code_context))
        .collect::<Vec<_>>();
    let latest_status = signals
        .last()
        .and_then(|signal| signal.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string);
    json!({
        "schema_version": 1,
        "source": "task_journal_step_trace",
        "validation_step_count": signals.len(),
        "latest_status": latest_status,
        "signals": signals,
    })
}

fn validation_signal_from_step(step: &TaskJournalStepTrace, code_context: bool) -> Option<Value> {
    if let Some(error) = step
        .error_excerpt
        .as_deref()
        .and_then(crate::skills::parse_structured_skill_error)
    {
        if matches!(
            error.error_kind.as_str(),
            "validation_failed" | "validation_inconclusive"
        ) {
            return Some(json!({
                "step_id": &step.step_id,
                "source": "step_error",
                "status": error.error_kind.as_str(),
                "status_code": error.error_kind.as_str(),
                "message_key": error
                    .extra
                    .as_ref()
                    .and_then(|extra| extra.get("message_key"))
                    .and_then(Value::as_str),
            }));
        }
    }

    if let Some(validation) = step
        .output_excerpt
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text.trim()).ok())
        .and_then(|value| {
            value
                .get("validation_result")
                .or_else(|| value.get("validation"))
                .cloned()
        })
    {
        let status = validation
            .get("status")
            .or_else(|| validation.get("status_code"))
            .and_then(Value::as_str)
            .unwrap_or("present");
        return Some(json!({
            "step_id": &step.step_id,
            "source": "step_output",
            "status": status,
            "status_code": validation
                .get("status_code")
                .and_then(Value::as_str)
                .unwrap_or(status),
            "message_key": validation.get("message_key").and_then(Value::as_str),
        }));
    }

    if code_context
        && step.status == crate::executor::StepExecutionStatus::Ok
        && matches!(step.skill.as_str(), "run_cmd" | "process_basic")
    {
        return Some(json!({
            "step_id": &step.step_id,
            "source": "step_status",
            "status": "passed",
            "status_code": "validation_command_ok",
            "message_key": Value::Null,
        }));
    }

    None
}

fn journal_has_code_or_test_artifact_step(journal: &TaskJournal) -> bool {
    journal.step_results.iter().any(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            return false;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(output.trim()) else {
            return false;
        };
        let extra = value
            .get("extra")
            .filter(|extra| extra.is_object())
            .unwrap_or(&value);
        let action = extra
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if !matches!(
            action,
            "write_text" | "append_text" | "read_range" | "read_text_range" | "grep_text"
        ) {
            return false;
        }
        ["resolved_path", "effective_path", "path"]
            .iter()
            .find_map(|field| extra.get(*field).and_then(Value::as_str))
            .is_some_and(path_looks_like_code_or_test)
    })
}

fn path_looks_like_code_or_test(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    let extension = Path::new(&normalized)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    matches!(
        extension.as_str(),
        "py" | "rs"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "go"
            | "java"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
            | "swift"
            | "kt"
            | "scala"
            | "sh"
            | "sql"
    )
}

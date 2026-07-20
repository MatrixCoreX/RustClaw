use serde_json::{json, Value};

use crate::AppState;

fn answer_verifier_requests_filtered_entry(journal: &crate::task_journal::TaskJournal) -> bool {
    journal
        .answer_verifier_summary
        .as_ref()
        .filter(|summary| summary.high_confidence_retry_gap())
        .is_some_and(|summary| {
            summary
                .missing_evidence_fields
                .iter()
                .any(|field| field.trim() == "filtered_entry")
        })
}

fn log_path_from_read_range_value(value: &Value) -> Option<String> {
    if value.get("action").and_then(Value::as_str) != Some("read_range") {
        return None;
    }
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("log"))
        .then(|| path.to_string())
}

fn read_range_log_excerpt_from_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    for candidate in [&value, value.get("extra").unwrap_or(&Value::Null)] {
        if log_path_from_read_range_value(candidate).is_none() {
            continue;
        }
        if let Some(excerpt) = candidate
            .get("excerpt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|excerpt| !excerpt.is_empty())
        {
            return Some(excerpt.to_string());
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilteredLogEntry {
    severity_rank: u8,
    level: &'static str,
    line_no: Option<u64>,
    text: String,
}

fn parse_log_alert_line(line: &str) -> Option<FilteredLogEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (line_no, text) = line
        .split_once('|')
        .and_then(|(prefix, rest)| {
            prefix
                .trim()
                .parse::<u64>()
                .ok()
                .map(|line_no| (Some(line_no), rest.trim()))
        })
        .unwrap_or((None, line));
    let level = text.split_whitespace().find_map(|token| {
        match token.trim_matches(|ch: char| !ch.is_ascii_alphabetic()) {
            "ERROR" => Some((2, "ERROR")),
            "WARN" => Some((1, "WARN")),
            _ => None,
        }
    })?;
    Some(FilteredLogEntry {
        severity_rank: level.0,
        level: level.1,
        line_no,
        text: text.to_string(),
    })
}

fn most_notable_log_alert_entry(excerpt: &str) -> Option<FilteredLogEntry> {
    excerpt
        .lines()
        .filter_map(parse_log_alert_line)
        .max_by(|left, right| {
            left.severity_rank
                .cmp(&right.severity_rank)
                .then_with(|| left.line_no.unwrap_or(0).cmp(&right.line_no.unwrap_or(0)))
        })
}

fn filtered_log_entry_answer(entry: &FilteredLogEntry) -> String {
    let line = entry
        .line_no
        .map(|line_no| line_no.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "log.filtered_entry.level={}; log.filtered_entry.line={}; log.filtered_entry.text={}",
        entry.level, line, entry.text
    )
}

pub(super) fn deterministic_filtered_log_entry_recovery(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if !answer_verifier_requests_filtered_entry(journal) {
        return None;
    }
    journal.step_results.iter().rev().find_map(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            return None;
        }
        let excerpt = step
            .output_excerpt
            .as_deref()
            .and_then(read_range_log_excerpt_from_output)?;
        most_notable_log_alert_entry(&excerpt).map(|entry| filtered_log_entry_answer(&entry))
    })
}

fn exact_tail_read_route_allows_failure_recovery(route: &crate::IntentOutputContract) -> bool {
    route.requests_exact_command_output()
        && route.response_shape == crate::OutputResponseShape::Strict
        && route.requires_content_evidence
        && !route.delivery_required
}

fn exact_tail_read_finalizer_has_qualified_evidence(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal.finalizer_summary.as_ref().is_some_and(|summary| {
        summary.contract_ok
            && matches!(
                summary.disposition,
                Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
            )
            && summary.used_evidence_ids_count > 0
    })
}

fn exact_tail_read_answer_from_value(value: &Value, prefer_english: bool) -> Option<String> {
    if let Some(answer) = exact_tail_read_answer_from_flat_value(value, prefer_english) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(|extra| exact_tail_read_answer_from_value(extra, prefer_english))
}

fn exact_tail_read_answer_from_flat_value(value: &Value, prefer_english: bool) -> Option<String> {
    if !matches!(
        value.get("action").and_then(Value::as_str),
        Some("read_range" | "read_text_range")
    ) || value.get("mode").and_then(Value::as_str) != Some("tail")
    {
        return None;
    }
    let requested_n = value
        .get("requested_n")
        .or_else(|| value.get("n"))
        .or_else(|| value.get("count"))
        .and_then(Value::as_u64)?;
    if requested_n == 0 || requested_n > 50 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(Value::as_str)
        .filter(|excerpt| !excerpt.trim().is_empty())?;
    let mut candidate = value.clone();
    let obj = candidate.as_object_mut()?;
    obj.insert(
        "action".to_string(),
        Value::String("read_range".to_string()),
    );
    if !obj.contains_key("requested_n") {
        obj.insert("requested_n".to_string(), json!(requested_n));
    }
    crate::agent_engine::observed_output::tail_read_range_direct_answer_candidate(
        &candidate.to_string(),
        prefer_english,
    )
}

fn exact_tail_read_answer_from_step_output(output: &str, prefer_english: bool) -> Option<String> {
    serde_json::from_str::<Value>(output.trim())
        .ok()
        .and_then(|value| exact_tail_read_answer_from_value(&value, prefer_english))
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}

pub(super) fn deterministic_exact_tail_read_failure_recovery(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if !exact_tail_read_route_allows_failure_recovery(route_result)
        || !exact_tail_read_finalizer_has_qualified_evidence(journal)
    {
        return None;
    }
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let prefer_english =
        crate::fallback::fallback_prefers_english_for_language_hint(state, &language_hint);
    journal.step_results.iter().rev().find_map(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            return None;
        }
        step.output_excerpt
            .as_deref()
            .and_then(|output| exact_tail_read_answer_from_step_output(output, prefer_english))
    })
}

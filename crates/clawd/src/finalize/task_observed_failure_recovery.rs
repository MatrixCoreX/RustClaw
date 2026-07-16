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

fn answer_verifier_requests_content_excerpt(journal: &crate::task_journal::TaskJournal) -> bool {
    journal
        .answer_verifier_summary
        .as_ref()
        .filter(|summary| summary.high_confidence_retry_gap())
        .is_some_and(|summary| {
            summary
                .missing_evidence_fields
                .iter()
                .any(|field| field.trim() == "content_excerpt")
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
        if step.status != crate::executor::StepExecutionStatus::Ok
            || !matches!(step.skill.as_str(), "system_basic" | "fs_basic")
        {
            return None;
        }
        let excerpt = step
            .output_excerpt
            .as_deref()
            .and_then(read_range_log_excerpt_from_output)?;
        most_notable_log_alert_entry(&excerpt).map(|entry| filtered_log_entry_answer(&entry))
    })
}

fn content_tail_read_route_allows_failure_recovery(
    route_result: &crate::IntentOutputContract,
) -> bool {
    let contract = route_result.clone();
    !matches!(
        contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) && contract.requires_content_evidence
        && !contract.delivery_required
        && route_result.semantic_kind_is_any(&[
            crate::OutputSemanticKind::ContentExcerptSummary,
            crate::OutputSemanticKind::ExcerptKindJudgment,
        ])
}

fn content_tail_read_answer_from_step_output(output: &str, prefer_english: bool) -> Option<String> {
    crate::finalize::selected_tail_read_range_line_from_step_output(output, prefer_english)
        .or_else(|| raw_tail_read_answer_from_step_output(output, prefer_english))
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}

pub(super) fn deterministic_content_tail_read_failure_recovery(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if !content_tail_read_route_allows_failure_recovery(route_result)
        || !answer_verifier_requests_content_excerpt(journal)
    {
        return None;
    }
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let prefer_english =
        crate::fallback::fallback_prefers_english_for_language_hint(state, &language_hint);
    journal.step_results.iter().rev().find_map(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || !matches!(step.skill.as_str(), "system_basic" | "fs_basic")
        {
            return None;
        }
        step.output_excerpt
            .as_deref()
            .and_then(|output| content_tail_read_answer_from_step_output(output, prefer_english))
    })
}

fn raw_tail_read_route_allows_failure_recovery(route_result: &crate::IntentOutputContract) -> bool {
    let contract = route_result.clone();
    route_result.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && contract.response_shape == crate::OutputResponseShape::Strict
        && contract.requires_content_evidence
        && !contract.delivery_required
}

fn raw_tail_read_finalizer_has_qualified_evidence(
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

fn raw_tail_read_answer_from_value(value: &Value, prefer_english: bool) -> Option<String> {
    if let Some(answer) = raw_tail_read_answer_from_flat_value(value, prefer_english) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(|extra| raw_tail_read_answer_from_value(extra, prefer_english))
}

fn raw_tail_read_answer_from_flat_value(value: &Value, prefer_english: bool) -> Option<String> {
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

fn raw_tail_read_answer_from_step_output(output: &str, prefer_english: bool) -> Option<String> {
    serde_json::from_str::<Value>(output.trim())
        .ok()
        .and_then(|value| raw_tail_read_answer_from_value(&value, prefer_english))
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}

pub(super) fn deterministic_raw_tail_read_failure_recovery(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if !raw_tail_read_route_allows_failure_recovery(route_result)
        || !raw_tail_read_finalizer_has_qualified_evidence(journal)
    {
        return None;
    }
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let prefer_english =
        crate::fallback::fallback_prefers_english_for_language_hint(state, &language_hint);
    journal.step_results.iter().rev().find_map(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || !matches!(step.skill.as_str(), "system_basic" | "fs_basic")
        {
            return None;
        }
        step.output_excerpt
            .as_deref()
            .and_then(|output| raw_tail_read_answer_from_step_output(output, prefer_english))
    })
}

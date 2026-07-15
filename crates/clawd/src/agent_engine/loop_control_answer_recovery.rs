use super::*;

#[path = "loop_control_answer_recovery/http_health.rs"]
mod http_health;
#[path = "loop_control_answer_recovery/observed_rewrite.rs"]
mod observed_rewrite;
#[path = "loop_control_answer_recovery/structured_evidence_table.rs"]
mod structured_evidence_table;
#[path = "loop_control_answer_recovery/structured_listing.rs"]
mod structured_listing;
#[path = "loop_control_answer_recovery/terminal_format.rs"]
mod terminal_format;

pub(super) use http_health::try_recover_http_health_answer_verifier_gap;
#[cfg(test)]
pub(super) use observed_rewrite::answer_verifier_gap_has_observed_content_evidence;
pub(super) use observed_rewrite::{
    answer_verifier_gap_requests_observed_content_rewrite,
    try_rewrite_answer_verifier_gap_with_observed_evidence,
};
pub(super) use structured_evidence_table::try_recover_structured_evidence_table_answer_verifier_gap;
pub(super) use structured_listing::try_recover_structured_listing_answer_verifier_gap;
pub(super) use terminal_format::prefer_terminal_model_answer_for_verifier_candidate;
use terminal_format::terminal_model_output_format_gap_satisfies_contract;

pub(super) fn answer_verifier_retry_summary<'a>(
    reply: &'a AskReply,
    route_result: Option<&RouteResult>,
) -> Option<&'a crate::task_journal::TaskJournalAnswerVerifierSummary> {
    if reply_final_status_is_clarify(reply) {
        return None;
    }
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())?;
    if reply.should_fail_task && !reply_failure_is_recoverable_answer_verifier_gap(reply, summary) {
        return None;
    }
    if answer_verifier_gap_is_structurally_satisfied(reply, route_result) {
        return None;
    }
    if answer_verifier_gap_requires_user_locator_disambiguation(reply, route_result, summary) {
        return None;
    }
    if answer_verifier_gap_has_confirmed_missing_file_delivery(reply, route_result, summary) {
        return None;
    }
    summary.high_confidence_retry_gap().then_some(summary)
}

fn reply_failure_is_recoverable_answer_verifier_gap(
    reply: &AskReply,
    summary: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> bool {
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    if matches!(
        journal.final_status,
        Some(
            crate::task_journal::TaskJournalFinalStatus::Clarify
                | crate::task_journal::TaskJournalFinalStatus::ResumeFailure
        )
    ) {
        return false;
    }
    matches!(
        journal.final_failure_attribution.as_deref(),
        None | Some("answer_verifier_gap") | Some("contract_gap")
    )
}

pub(super) fn suppress_answer_verifier_retry_if_structurally_satisfied(
    reply: &mut AskReply,
    route_result: Option<&RouteResult>,
) -> bool {
    if !answer_verifier_gap_is_structurally_satisfied(reply, route_result) {
        return false;
    }
    let Some(journal) = reply.task_journal.as_mut() else {
        return false;
    };
    let Some(summary) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    info!(
        "answer_verifier_retry_suppressed_structural_satisfaction reason={}",
        crate::truncate_for_log(&summary.answer_incomplete_reason)
    );
    journal.answer_verifier_summary = None;
    true
}

pub(super) fn suppress_answer_verifier_retry_if_confirmed_missing_file_delivery(
    reply: &mut AskReply,
    route_result: Option<&RouteResult>,
) -> bool {
    let Some(summary) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !answer_verifier_gap_has_confirmed_missing_file_delivery(reply, route_result, summary) {
        return false;
    }
    let reason = summary.answer_incomplete_reason.clone();
    let Some(journal) = reply.task_journal.as_mut() else {
        return false;
    };
    info!(
        "answer_verifier_retry_suppressed_missing_file_delivery reason={}",
        crate::truncate_for_log(&reason)
    );
    journal.answer_verifier_summary = None;
    true
}

pub(super) fn suppress_answer_verifier_retry_if_user_locator_disambiguation(
    reply: &mut AskReply,
    route_result: Option<&RouteResult>,
) -> bool {
    let Some(summary) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !answer_verifier_gap_requires_user_locator_disambiguation(reply, route_result, summary) {
        return false;
    }
    let reason = summary.answer_incomplete_reason.clone();
    let Some(journal) = reply.task_journal.as_mut() else {
        return false;
    };
    info!(
        "answer_verifier_retry_suppressed_locator_disambiguation reason={}",
        crate::truncate_for_log(&reason)
    );
    journal.answer_verifier_summary = None;
    true
}

fn mark_answer_verifier_recovery_success(
    journal: &mut crate::task_journal::TaskJournal,
    answer: &str,
) {
    journal.answer_verifier_summary = None;
    journal.record_final_answer(answer);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    journal.record_final_stop_signal(
        crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL,
    );
}

fn answer_verifier_gap_requires_user_locator_disambiguation(
    reply: &AskReply,
    route_result: Option<&RouteResult>,
    summary: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> bool {
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    if summary.missing_evidence_fields.len() != 1
        || summary.missing_evidence_fields.first().map(String::as_str) != Some("path")
    {
        return false;
    }
    let Some(route) = route_result else {
        return false;
    };
    if !route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::FileToken
        || route.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
    {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    journal
        .step_results
        .iter()
        .any(step_has_non_unique_file_search_candidates)
}

fn answer_verifier_gap_has_confirmed_missing_file_delivery(
    reply: &AskReply,
    route_result: Option<&RouteResult>,
    summary: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> bool {
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    if !summary.missing_evidence_fields.iter().any(|field| {
        matches!(
            field.as_str(),
            "path" | "content_excerpt" | "any_of(candidates|count|path)"
        )
    }) {
        return false;
    }
    let Some(route) = route_result else {
        return false;
    };
    if !route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::FileToken
    {
        return false;
    }
    if final_user_answer_candidate(reply).is_some_and(answer_has_file_delivery_token) {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    journal
        .step_results
        .iter()
        .any(step_has_missing_file_search_evidence)
}

fn step_has_missing_file_search_evidence(step: &crate::task_journal::TaskJournalStepTrace) -> bool {
    step.output_excerpt
        .as_deref()
        .is_some_and(output_excerpt_has_missing_file_search_evidence)
        || step
            .error_excerpt
            .as_deref()
            .is_some_and(step_error_has_missing_file_evidence)
}

fn output_excerpt_has_missing_file_search_evidence(output: &str) -> bool {
    if output.trim().eq_ignore_ascii_case("NOT_FOUND") {
        return true;
    }
    serde_json::from_str::<serde_json::Value>(output)
        .ok()
        .is_some_and(|value| output_value_has_missing_file_search_evidence(&value))
}

fn output_value_has_missing_file_search_evidence(value: &serde_json::Value) -> bool {
    let locator_found_nothing = value
        .get("action")
        .and_then(|v| v.as_str())
        .is_some_and(|action| matches!(action, "find_name" | "find_path"))
        && value.get("count").and_then(|v| v.as_i64()) == Some(0)
        && ["results", "matches"].iter().any(|field| {
            value
                .get(field)
                .and_then(|v| v.as_array())
                .is_some_and(|items| items.is_empty())
        });
    if locator_found_nothing {
        return true;
    }
    value
        .get("extra")
        .is_some_and(output_value_has_missing_file_search_evidence)
}

fn answer_has_file_delivery_token(answer: &str) -> bool {
    answer
        .lines()
        .any(|line| crate::finalize::parse_delivery_file_token(line.trim()).is_some())
}

fn step_error_has_missing_file_evidence(error: &str) -> bool {
    let trimmed = error.trim();
    trimmed.starts_with("__RC_READ_FILE_NOT_FOUND__:")
        || crate::skills::parse_structured_skill_error(trimmed)
            .is_some_and(|structured| structured.error_kind == "not_found")
}

fn step_has_non_unique_file_search_candidates(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    if step.status != crate::executor::StepExecutionStatus::Ok {
        return false;
    }
    let Some(output) = step.output_excerpt.as_deref().map(str::trim) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
        return false;
    };
    let payload = value.get("extra").unwrap_or(&value);
    if payload.get("action").and_then(serde_json::Value::as_str) != Some("find_name") {
        return false;
    }
    let results_len = payload
        .get("results")
        .and_then(serde_json::Value::as_array)
        .map_or(0, Vec::len);
    let count = payload
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(results_len);
    count > 1 || results_len > 1
}

pub(super) fn answer_verifier_gap_is_structurally_satisfied(
    reply: &AskReply,
    route_result: Option<&RouteResult>,
) -> bool {
    let Some(summary) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    if summary
        .answer_incomplete_reason
        .starts_with("post_write_content_evidence_required")
    {
        return false;
    }
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract_marker_is(crate::OutputSemanticKind::QuantityComparison) {
        return quantity_comparison_reply_has_derived_numeric_answer(reply);
    }
    if terminal_content_access_blocker_reply_satisfies_contract(reply, route) {
        return true;
    }
    if terminal_model_output_format_gap_satisfies_contract(reply, route) {
        return true;
    }
    if summary.missing_evidence_fields.iter().any(|field| {
        matches!(
            field.as_str(),
            "output_format" | "unsupported_claims" | "candidates"
        )
    }) {
        return false;
    }
    if let (Some(journal), Some(answer)) = (
        reply.task_journal.as_ref(),
        final_user_answer_candidate(reply),
    ) {
        return crate::answer_verifier::structurally_satisfies_answer_contract(
            route, journal, answer,
        );
    }
    false
}

pub(super) fn terminal_content_access_blocker_reply_satisfies_contract(
    reply: &AskReply,
    route: &RouteResult,
) -> bool {
    if !route.output_contract.requires_content_evidence {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    let Some(summary) = journal.finalizer_summary.as_ref() else {
        return false;
    };
    if summary.disposition != Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        || summary.completion_ok != Some(true)
        || summary.grounded_ok != Some(true)
    {
        return false;
    }
    journal
        .step_results
        .iter()
        .rev()
        .any(step_has_terminal_content_access_blocker)
}

pub(super) fn step_has_terminal_content_access_blocker(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    if step.status != crate::executor::StepExecutionStatus::Error {
        return false;
    }
    let Some(error) = step.error_excerpt.as_deref().map(str::trim) else {
        return false;
    };
    if error.is_empty() {
        return false;
    }
    if let Some(policy_block) = crate::skills::parse_policy_block_error(error) {
        return matches!(
            policy_block.reason_code.as_str(),
            "path_outside_workspace" | "path_parent_traversal"
        );
    }
    let Some(structured) = crate::skills::parse_structured_skill_error(error) else {
        return false;
    };
    if structured.error_kind != "permission_denied" {
        return false;
    }
    let effective_skill = if structured.skill.trim().is_empty() {
        step.skill.as_str()
    } else {
        structured.skill.as_str()
    };
    matches!(
        effective_skill.to_ascii_lowercase().as_str(),
        "fs_basic" | "system_basic" | "read_file" | "list_dir"
    )
}

pub(super) fn final_user_answer_candidate(reply: &AskReply) -> Option<&str> {
    reply
        .messages
        .iter()
        .rev()
        .map(String::as_str)
        .find(|message| {
            let trimmed = message.trim();
            !trimmed.is_empty() && !crate::finalize::is_execution_summary_message(trimmed)
        })
        .or_else(|| {
            let trimmed = reply.text.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })
}

pub(super) fn collect_size_bytes_from_json(value: &serde_json::Value, out: &mut Vec<u64>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(size) = map
                .get("size_bytes")
                .or_else(|| map.get("total_size_bytes"))
                .and_then(|value| value.as_u64())
            {
                out.push(size);
            }
            for value in map.values() {
                collect_size_bytes_from_json(value, out);
            }
        }
        serde_json::Value::Array(items) => {
            for value in items {
                collect_size_bytes_from_json(value, out);
            }
        }
        _ => {}
    }
}

pub(super) fn observed_size_bytes(reply: &AskReply) -> Vec<u64> {
    let mut sizes = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return sizes;
    };
    for step in &journal.step_results {
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
            collect_size_bytes_from_json(&value, &mut sizes);
        }
    }
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

pub(super) fn numeric_literals(text: &str) -> Vec<f64> {
    let mut values = Vec::new();
    let mut token = String::new();
    let mut has_digit = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == ',' || ch == '.' {
            if ch.is_ascii_digit() {
                has_digit = true;
            }
            token.push(ch);
            continue;
        }
        if has_digit {
            push_numeric_literal(&mut values, &token);
        }
        token.clear();
        has_digit = false;
    }
    if has_digit {
        push_numeric_literal(&mut values, &token);
    }
    values
}

pub(super) fn push_numeric_literal(values: &mut Vec<f64>, token: &str) {
    let normalized = token.trim_matches('.').replace(',', "");
    if normalized.is_empty() || normalized == "." {
        return;
    }
    if let Ok(value) = normalized.parse::<f64>() {
        values.push(value);
    }
}

pub(super) fn quantity_comparison_reply_has_derived_numeric_answer(reply: &AskReply) -> bool {
    let observed_sizes = observed_size_bytes(reply);
    if observed_sizes.len() < 2 {
        return false;
    }
    let Some(answer) = final_user_answer_candidate(reply) else {
        return false;
    };
    numeric_literals(answer).into_iter().any(|number| {
        !observed_sizes
            .iter()
            .any(|size| (number - *size as f64).abs() < 0.000_001)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LogAnalyzeFinding {
    pub(super) path: String,
    pub(super) keyword_counts: Vec<(String, u64)>,
    pub(super) total_hits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StructuredSearchFinding {
    pub(super) action: String,
    pub(super) count: usize,
    pub(super) results: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StructuredCountFinding {
    pub(super) path: Option<String>,
    pub(super) total: u64,
    pub(super) files: Option<u64>,
    pub(super) dirs: Option<u64>,
    pub(super) hidden: Option<u64>,
    pub(super) recursive: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RssNewsItem {
    pub(super) title: String,
    pub(super) source_host: String,
    pub(super) date: Option<String>,
}

pub(super) fn try_recover_log_analyze_answer_verifier_gap(
    user_text: &str,
    reply: &mut AskReply,
) -> bool {
    let findings = observed_log_analyze_findings(reply);
    if findings.is_empty() {
        return false;
    }
    let answer = deterministic_log_analyze_summary_text(user_text, &findings);
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!(
        "answer_verifier_retry_exhausted_recovered_with_log_analyze_summary findings={}",
        findings.len()
    );
    true
}

pub(super) fn try_recover_structured_count_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    user_text: &str,
    reply: &mut AskReply,
) -> bool {
    if !route_result.is_some_and(|route| {
        route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount)
    }) {
        return false;
    }
    let Some(finding) = observed_structured_count_findings(reply).into_iter().next() else {
        return false;
    };
    let answer = deterministic_structured_count_summary_text(user_text, &finding);
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!(
        "answer_verifier_retry_exhausted_recovered_with_structured_count total={}",
        finding.total
    );
    true
}

pub(super) fn try_recover_structured_search_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    user_text: &str,
    reply: &mut AskReply,
) -> bool {
    if !route_allows_structured_search_recovery(route_result) {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap() {
        return false;
    }
    if !structured_search_verifier_requests_full_candidates(verifier) {
        return false;
    }
    let Some(finding) = observed_structured_search_findings(reply)
        .into_iter()
        .max_by(|left, right| {
            left.count
                .cmp(&right.count)
                .then_with(|| left.results.len().cmp(&right.results.len()))
                .then_with(|| right.action.cmp(&left.action))
        })
    else {
        return false;
    };
    if finding.results.is_empty() || finding.count > finding.results.len() {
        return false;
    }
    let answer = deterministic_structured_search_summary_text(user_text, &finding);
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!(
        "answer_verifier_retry_exhausted_recovered_with_structured_search_results action={} count={}",
        finding.action, finding.count
    );
    true
}

pub(super) fn try_recover_rss_news_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route_is_rss_news_fetch(route) {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap() || !rss_verifier_requests_source_grounding(verifier) {
        return false;
    }
    if !reply.task_journal.as_ref().is_some_and(|journal| {
        crate::task_journal::evidence_coverage_for_route(route, journal).is_complete()
    }) {
        return false;
    }
    let items = observed_rss_news_items(reply);
    if items.is_empty() {
        return false;
    }
    apply_rss_news_items_answer(reply, &items);
    info!(
        "answer_verifier_retry_exhausted_recovered_with_rss_structured_items count={}",
        items.len()
    );
    true
}

pub(super) fn try_preserve_rss_source_hosts_from_structured_evidence(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route_is_rss_news_fetch(route) {
        return false;
    }
    if !reply.task_journal.as_ref().is_some_and(|journal| {
        journal
            .answer_verifier_summary
            .as_ref()
            .is_none_or(|verifier| verifier.pass)
            && crate::task_journal::evidence_coverage_for_route(route, journal).is_complete()
    }) {
        return false;
    }
    let items = observed_rss_news_items(reply);
    if items.is_empty() || rss_answer_contains_observed_source_hosts(&reply.text, &items) {
        return false;
    }
    apply_rss_news_items_answer(reply, &items);
    info!(
        "rss_source_host_fidelity_recovered_with_structured_items count={}",
        items.len()
    );
    true
}

pub(super) fn apply_rss_news_items_answer(reply: &mut AskReply, items: &[RssNewsItem]) {
    let answer = deterministic_rss_news_items_text(items);
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
}

pub(super) fn route_allows_structured_search_recovery(
    route_result: Option<&crate::RouteResult>,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::FileNames,
        crate::OutputSemanticKind::DirectoryNames,
        crate::OutputSemanticKind::FilePaths,
    ])
}

pub(super) fn try_recover_document_heading_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    if !route_result.is_some_and(route_allows_document_heading_recovery) {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap() {
        return false;
    }
    if !route_result.is_some_and(|route| {
        route.output_contract_marker_is(crate::OutputSemanticKind::DocumentHeading)
            || verifier
                .missing_evidence_fields
                .iter()
                .any(|field| field == "output_format")
    }) {
        return false;
    }
    if !reply.task_journal.as_ref().is_some_and(|journal| {
        route_result.is_some_and(|route| {
            crate::task_journal::evidence_coverage_for_route(route, journal).is_complete()
        })
    }) {
        return false;
    }
    let Some(answer) = observed_markdown_heading(reply) else {
        return false;
    };
    let verifier_summary = verifier.clone();
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.rollout_attribution.push(
            crate::task_journal::TaskJournalRolloutAttribution::document_heading_answer_verifier_recovery(
                Some(&verifier_summary),
            ),
        );
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_recovered_with_document_heading");
    true
}

pub(super) fn route_allows_document_heading_recovery(route: &crate::RouteResult) -> bool {
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::DocumentHeading) {
        return true;
    }
    route.output_contract_is_unclassified()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.locator_hint.trim().is_empty()
        && route
            .has_route_reason_machine_marker("session_alias_locator_prebound_from_current_request")
}

pub(super) fn observed_markdown_heading(reply: &AskReply) -> Option<String> {
    reply
        .task_journal
        .as_ref()?
        .step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .find_map(markdown_heading_from_output_excerpt)
}

pub(super) fn markdown_heading_from_output_excerpt(output: &str) -> Option<String> {
    let mut sources = Vec::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        collect_heading_candidate_texts(&value, &mut sources);
    }
    sources.push(output.to_string());
    sources
        .into_iter()
        .find_map(|source| markdown_heading_from_text(&source))
}

pub(super) fn collect_heading_candidate_texts(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(obj) => {
            for key in ["excerpt", "content_excerpt", "text"] {
                if let Some(text) = obj
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    out.push(text.to_string());
                }
            }
            for key in ["extra", "result", "data"] {
                if let Some(child) = obj.get(key) {
                    collect_heading_candidate_texts(child, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_heading_candidate_texts(item, out);
            }
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
                if trimmed.starts_with('{') {
                    if let Ok(nested) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        collect_heading_candidate_texts(&nested, out);
                    }
                }
            }
        }
        _ => {}
    }
}

pub(super) fn markdown_heading_from_text(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| markdown_heading_from_line(line.trim()))
        .next()
}

pub(super) fn markdown_heading_from_line(line: &str) -> Option<String> {
    let line = strip_read_range_line_prefix(line);
    let trimmed = line.trim_start();
    let hash_count = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hash_count) {
        return None;
    }
    let rest = trimmed.get(hash_count..)?.trim();
    if rest.is_empty() {
        return None;
    }
    let heading = rest
        .trim_end_matches('#')
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'));
    (!heading.is_empty()).then(|| heading.to_string())
}

pub(super) fn strip_read_range_line_prefix(line: &str) -> &str {
    let Some((prefix, rest)) = line.split_once('|') else {
        return line;
    };
    let prefix = prefix.trim();
    if prefix.is_empty() || prefix.len() > 6 || !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return line;
    }
    rest
}

pub(super) fn try_recover_content_excerpt_summary_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route_allows_synthesis_recovery(route) {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap() {
        return false;
    }
    if !verifier.missing_evidence_fields.iter().any(|field| {
        field == "content_excerpt" || field == "any_of(command_output|content_excerpt|field_value)"
    }) {
        return false;
    }
    if !reply.task_journal.as_ref().is_some_and(|journal| {
        crate::task_journal::evidence_coverage_for_route(route, journal).is_complete()
    }) {
        return false;
    }
    let Some(answer) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| {
            journal
                .step_results
                .iter()
                .rev()
                .find(|step| {
                    step.skill == "synthesize_answer"
                        && step.status == crate::executor::StepExecutionStatus::Ok
                        && step
                            .output_excerpt
                            .as_deref()
                            .is_some_and(|text| !text.trim().is_empty())
                })
                .and_then(|step| step.output_excerpt.as_deref())
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .filter(|text| !crate::finalize::looks_like_planner_artifact(text))
        .filter(|text| !crate::finalize::looks_like_internal_trace_artifact(text))
        .filter(|text| !crate::finalize::is_execution_summary_message(text))
        .map(ToString::to_string)
    else {
        return false;
    };
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_content_excerpt_summary_synthesis");
    true
}

pub(super) fn try_recover_latest_synthesis_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    let Some(verifier) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    if !verifier.high_confidence_retry_gap() {
        return false;
    }
    if !crate::task_journal::evidence_coverage_for_route(route, journal).is_complete() {
        return false;
    }
    if verifier_requires_structured_visible_rewrite(verifier) {
        return false;
    }
    if super::loop_control_post_write_evidence_guard::journal_has_code_write_followed_by_failed_validation(journal) {
        return false;
    }
    let Some(candidate) = latest_recoverable_terminal_answer(route, journal, reply) else {
        return false;
    };
    let answer = candidate.answer;
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer.clone();
    reply.messages = vec![answer];
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_latest_synthesis");
    true
}

fn verifier_requires_structured_visible_rewrite(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> bool {
    verifier.missing_evidence_fields.iter().any(|field| {
        matches!(
            field.as_str(),
            "output_format" | "field_value" | "candidates"
        )
    })
}

struct TerminalAnswerCandidate {
    source_skill: &'static str,
    answer: String,
}

fn latest_recoverable_terminal_answer(
    route: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    reply: &AskReply,
) -> Option<TerminalAnswerCandidate> {
    journal
        .step_results
        .iter()
        .rev()
        .filter_map(terminal_answer_candidate_from_step)
        .find(|candidate| {
            latest_terminal_candidate_can_recover_answer_gap(route, journal, reply, candidate)
        })
}

fn terminal_answer_candidate_from_step(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> Option<TerminalAnswerCandidate> {
    if !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        || step.status != crate::executor::StepExecutionStatus::Ok
    {
        return None;
    }
    let source_skill = match step.skill.as_str() {
        "respond" => "respond",
        "synthesize_answer" => "synthesize_answer",
        _ => return None,
    };
    let answer = step.output_excerpt.as_deref()?.trim();
    if answer.is_empty()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
    {
        return None;
    }
    Some(TerminalAnswerCandidate {
        source_skill,
        answer: answer.to_string(),
    })
}

fn latest_terminal_candidate_can_recover_answer_gap(
    route: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    reply: &AskReply,
    candidate: &TerminalAnswerCandidate,
) -> bool {
    if crate::answer_verifier::structurally_satisfies_answer_contract(
        route,
        journal,
        &candidate.answer,
    ) {
        return true;
    }
    if route_requires_structural_terminal_recovery(route) {
        return false;
    }
    if reply.text.trim() == candidate.answer.trim() {
        return false;
    }
    if compound_observation_terminal_candidate_can_recover_answer_gap(
        route, journal, reply, candidate,
    ) {
        return true;
    }
    match candidate.source_skill {
        "respond" => route_allows_latest_respond_retry_recovery(route, journal),
        "synthesize_answer" => route_allows_latest_synthesis_retry_recovery(route, journal),
        _ => false,
    }
}

fn compound_observation_terminal_candidate_can_recover_answer_gap(
    route: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    reply: &AskReply,
    candidate: &TerminalAnswerCandidate,
) -> bool {
    route_allows_compound_terminal_retry_recovery(route)
        && verifier_gap_allows_compound_terminal_retry_recovery(journal)
        && journal_has_multiple_successful_observations(journal)
        && terminal_candidate_is_structurally_richer_than_current_reply(
            &candidate.answer,
            &reply.text,
        )
}

fn route_allows_compound_terminal_retry_recovery(route: &crate::RouteResult) -> bool {
    if route.output_contract.delivery_required
        || !matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Free
        )
    {
        return false;
    }
    crate::evidence_policy::final_answer_shape_for_route(route)
        .map(crate::evidence_policy::FinalAnswerShape::allows_model_language)
        .unwrap_or(true)
}

fn verifier_gap_allows_compound_terminal_retry_recovery(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let Some(summary) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    summary.high_confidence_retry_gap()
        && !verifier_requires_structured_visible_rewrite(summary)
        && summary.missing_evidence_fields.iter().any(|field| {
            matches!(
                field.as_str(),
                "content_excerpt"
                    | "observed_evidence"
                    | "source_evidence"
                    | "used_evidence"
                    | "used_evidence_ids"
                    | "evidence_quotes"
            )
        })
}

fn journal_has_multiple_successful_observations(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let mut observed = 0usize;
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
            || !step
                .output_excerpt
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
        {
            continue;
        }
        observed += 1;
        if observed >= 2 {
            return true;
        }
    }
    false
}

fn terminal_candidate_is_structurally_richer_than_current_reply(
    candidate: &str,
    current: &str,
) -> bool {
    let candidate = candidate.trim();
    let current = current.trim();
    if candidate.is_empty() || candidate == current {
        return false;
    }
    let candidate_lines = candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let current_lines = current
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let candidate_chars = candidate.chars().count();
    let current_chars = current.chars().count();
    candidate_lines >= 2
        && (candidate_chars > current_chars.saturating_add(32)
            || candidate_lines > current_lines.saturating_add(1))
}

fn route_requires_structural_terminal_recovery(route: &crate::RouteResult) -> bool {
    crate::evidence_policy::final_answer_shape_for_route(route)
        .is_some_and(|shape| !shape.allows_model_language())
}

fn route_allows_latest_synthesis_retry_recovery(
    route: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    if !crate::evidence_policy::final_answer_shape_for_route(route)
        .is_some_and(|shape| shape.allows_model_language())
    {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
            && step
                .output_excerpt
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    })
}

fn route_allows_latest_respond_retry_recovery(
    route: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    if route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || !route.output_contract.requires_content_evidence
    {
        return false;
    }
    if !crate::task_journal::evidence_coverage_for_route(route, journal).is_complete() {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
            && step
                .output_excerpt
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    })
}

pub(super) fn route_allows_synthesis_recovery(route: &crate::RouteResult) -> bool {
    if route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::ContentExcerptSummary,
        crate::OutputSemanticKind::ContentExcerptWithSummary,
        crate::OutputSemanticKind::WorkspaceProjectSummary,
    ]) {
        return true;
    }
    false
}

pub(super) fn try_recover_generic_path_content_read_range_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route.output_contract_is_unclassified()
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap()
        || !verifier
            .missing_evidence_fields
            .iter()
            .any(|field| matches!(field.as_str(), "path" | "content_excerpt"))
    {
        return false;
    }
    info!("answer_verifier_retry_exhausted_no_generic_path_content_raw_recovery");
    false
}

pub(super) fn try_recover_structured_scalar_output_format_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || !route.output_contract_is_unclassified()
    {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap()
        || !verifier
            .missing_evidence_fields
            .iter()
            .any(|field| matches!(field.as_str(), "field_value" | "output_format"))
    {
        return false;
    }
    let observed = observed_structured_read_scalar_values(reply);
    if observed.is_empty() {
        return false;
    }
    let mut candidates = quoted_scalar_values(&verifier.retry_instruction)
        .into_iter()
        .filter(|candidate| observed.iter().any(|value| value == candidate))
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    let [answer] = candidates.as_slice() else {
        return false;
    };
    let answer = answer.clone();
    if let Some(journal) = reply.task_journal.as_mut() {
        mark_answer_verifier_recovery_success(journal, &answer);
    }
    reply.text = answer.clone();
    reply.messages = vec![answer];
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_structured_scalar_value");
    true
}

pub(super) fn try_recover_machine_kv_summary_output_format_answer_verifier_gap(
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
            .all(|field| field == "output_format")
    {
        return false;
    }
    let observed_texts =
        crate::machine_kv_projection::observed_machine_text_fragments_from_journal(journal);
    let Some(answer) = crate::machine_kv_projection::requested_machine_kv_summary_from_observations(
        &journal.input_text,
        &observed_texts,
    ) else {
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
    info!("answer_verifier_retry_exhausted_recovered_with_machine_kv_summary");
    true
}

pub(super) fn quoted_scalar_values(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut escaped = false;
    for ch in text.chars() {
        if escaped {
            if in_quote {
                current.push(ch);
            }
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            if in_quote {
                let value = current.trim();
                if !value.is_empty() && value.lines().count() == 1 {
                    values.push(value.to_string());
                }
                current.clear();
            }
            in_quote = !in_quote;
            continue;
        }
        if in_quote {
            current.push(ch);
        }
    }
    values
}

pub(super) fn observed_structured_read_scalar_values(reply: &AskReply) -> Vec<String> {
    let Some(journal) = reply.task_journal.as_ref() else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        collect_structured_read_scalar_values_from_output(output, &mut values);
    }
    values.sort();
    values.dedup();
    values
}

pub(super) fn collect_structured_read_scalar_values_from_output(
    output: &str,
    values: &mut Vec<String>,
) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return;
    };
    collect_structured_read_scalar_values_from_json_output(&value, values);
}

pub(super) fn collect_structured_read_scalar_values_from_json_output(
    value: &serde_json::Value,
    values: &mut Vec<String>,
) {
    let action = value
        .get("action")
        .or_else(|| value.pointer("/extra/action"))
        .and_then(|value| value.as_str())
        .map(str::trim);
    if !matches!(action, Some("read_range" | "read_text_range")) {
        return;
    }
    for excerpt in [
        value.get("excerpt").and_then(|value| value.as_str()),
        value
            .pointer("/extra/excerpt")
            .and_then(|value| value.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        let body = read_range_excerpt_without_line_prefixes(excerpt);
        if let Ok(document) = serde_json::from_str::<serde_json::Value>(&body) {
            collect_json_scalar_values(&document, values);
        }
        collect_json_line_scalar_values(&body, values);
    }
}

pub(super) fn read_range_excerpt_without_line_prefixes(excerpt: &str) -> String {
    excerpt
        .lines()
        .map(strip_read_range_line_prefix)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn collect_json_line_scalar_values(body: &str, values: &mut Vec<String>) {
    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Some((key, raw_value)) = line.split_once(':') else {
            continue;
        };
        if serde_json::from_str::<serde_json::Value>(key.trim()).is_err() {
            continue;
        }
        let raw_value = raw_value.trim().trim_end_matches(',');
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw_value) {
            collect_json_scalar_values(&value, values);
        }
    }
}

pub(super) fn collect_json_scalar_values(value: &serde_json::Value, values: &mut Vec<String>) {
    match value {
        serde_json::Value::String(value) => {
            let value = value.trim();
            if !value.is_empty() && value.lines().count() == 1 {
                values.push(value.to_string());
            }
        }
        serde_json::Value::Number(value) => values.push(value.to_string()),
        serde_json::Value::Bool(value) => values.push(value.to_string()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_json_scalar_values(item, values);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_json_scalar_values(item, values);
            }
        }
        serde_json::Value::Null => {}
    }
}

fn route_is_rss_news_fetch(route: &crate::RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action(
        route,
        &["rss"],
        &["latest", "news", "fetch", "feed"],
    )
}

pub(super) fn rss_verifier_requests_source_grounding(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> bool {
    verifier
        .missing_evidence_fields
        .iter()
        .any(|field| matches!(field.as_str(), "source" | "source_host" | "field_value"))
}

pub(super) fn structured_search_verifier_requests_full_candidates(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> bool {
    verifier
        .missing_evidence_fields
        .iter()
        .any(|field| matches!(field.as_str(), "candidates" | "results" | "paths" | "files"))
}

pub(super) fn observed_log_analyze_findings(reply: &AskReply) -> Vec<LogAnalyzeFinding> {
    let mut findings = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return findings;
    };
    for step in &journal.step_results {
        if !step.skill.eq_ignore_ascii_case("log_analyze")
            || step.status != crate::executor::StepExecutionStatus::Ok
        {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(finding) = parse_log_analyze_finding(output) else {
            continue;
        };
        findings.push(finding);
    }
    findings.sort_by(|left, right| {
        right
            .total_hits
            .cmp(&left.total_hits)
            .then_with(|| left.path.cmp(&right.path))
    });
    findings
}

pub(super) fn observed_structured_count_findings(reply: &AskReply) -> Vec<StructuredCountFinding> {
    let mut findings = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return findings;
    };
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        if !matches!(step.skill.as_str(), "fs_basic" | "system_basic") {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(finding) = parse_structured_count_finding(output) else {
            continue;
        };
        findings.push(finding);
    }
    findings
}

pub(super) fn observed_structured_search_findings(
    reply: &AskReply,
) -> Vec<StructuredSearchFinding> {
    let mut findings = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return findings;
    };
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        if !matches!(
            step.skill.as_str(),
            "fs_basic" | "fs_search" | "system_basic"
        ) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(finding) = parse_structured_search_finding(output) else {
            continue;
        };
        findings.push(finding);
    }
    findings
}

pub(super) fn observed_rss_news_items(reply: &AskReply) -> Vec<RssNewsItem> {
    let mut items = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return items;
    };
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || !step.skill.eq_ignore_ascii_case("rss_fetch")
        {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(mut parsed_items) = parse_rss_news_items(output) else {
            continue;
        };
        items.append(&mut parsed_items);
    }
    let mut seen = std::collections::BTreeSet::new();
    items.retain(|item| {
        seen.insert((
            item.title.clone(),
            item.source_host.clone(),
            item.date.clone(),
        ))
    });
    items
}

pub(super) fn rss_answer_contains_observed_source_hosts(
    answer: &str,
    items: &[RssNewsItem],
) -> bool {
    let hosts = items
        .iter()
        .map(|item| item.source_host.as_str())
        .filter(|host| !host.trim().is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    !hosts.is_empty() && hosts.iter().all(|host| answer.contains(host))
}

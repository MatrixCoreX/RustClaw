use anyhow::Result;
use serde_json::{json, Value};
use tracing::{error, info, warn};

use crate::{repo, AppState};

#[path = "task_answer_verifier_failure.rs"]
mod task_answer_verifier_failure;
#[path = "task_config_guard_recovery.rs"]
mod task_config_guard_recovery;
#[path = "task_content_evidence_delivery.rs"]
mod task_content_evidence_delivery;
#[path = "task_deterministic_recovery.rs"]
mod task_deterministic_recovery;
#[path = "task_failure_lifecycle.rs"]
mod task_failure_lifecycle;
#[path = "task_machine_kv_summary.rs"]
mod task_machine_kv_summary;
#[path = "task_payload_helpers.rs"]
mod task_payload_helpers;
#[path = "task_resume.rs"]
mod task_resume;
#[path = "task_runtime_failure_payload.rs"]
mod task_runtime_failure_payload;
#[path = "task_structured_evidence_table.rs"]
mod task_structured_evidence_table;
#[path = "task_terminal_clarify.rs"]
mod task_terminal_clarify;
#[path = "task_tree_summary_recovery.rs"]
mod task_tree_summary_recovery;
#[path = "task_verifier_retry_commit.rs"]
mod task_verifier_retry_commit;

#[cfg(test)]
use task_answer_verifier_failure::{
    answer_text_is_machine_json_payload, answer_verifier_failure_default_payload,
    answer_verifier_failure_machine_line, answer_verifier_failure_missing_fields_text,
    answer_verifier_failure_observed_facts,
};
use task_answer_verifier_failure::{
    answer_verifier_failure_needs_user_message, compose_answer_verifier_failure_user_message,
    machine_payload_observed_facts,
};
use task_config_guard_recovery::deterministic_config_guard_candidates_recovery;
use task_content_evidence_delivery::{
    backfill_file_delivery_contract_from_journal, has_any_delivery_file_token,
    route_has_file_delivery_contract,
};
use task_deterministic_recovery::{
    mark_answer_verifier_recovered_by_deterministic_observed_evidence,
    recover_answer_verifier_gap_with_deterministic_machine_evidence,
    recover_raw_command_machine_field_final_answer,
};
use task_failure_lifecycle::failed_task_lifecycle_payload;
#[cfg(test)]
use task_machine_kv_summary::apply_requested_machine_kv_summary_to_final_answer;
use task_machine_kv_summary::recover_requested_machine_kv_summary_final_answer;
use task_payload_helpers::{
    answer_verifier_forces_task_failure, answer_verifier_should_force_task_failure,
    ask_result_payload, non_failure_final_status, normalize_existing_file_delivery_token_answer,
    should_skip_ask_memory_pair,
};
pub(crate) use task_resume::answer_verifier_retry_answer_has_required_machine_evidence;
use task_resume::{
    answer_verifier_retry_applicable, resume_context_execution_summary_messages,
    resume_failure_execution_failed_step_answer, resume_failure_is_missing_file_delivery_result,
    resume_failure_is_structured_service_status_result,
    resume_failure_is_unbound_path_lookup_clarify_result, retry_answer_after_verifier,
    text_looks_like_missing_file_target,
};
use task_runtime_failure_payload::ask_runtime_failure_machine_payload;
use task_structured_evidence_table::deterministic_structured_evidence_table_recovery;
use task_structured_evidence_table::verified_terminal_answer_after_verifier_pass;
use task_tree_summary_recovery::deterministic_tree_summary_rows_failure_recovery;
use task_verifier_retry_commit::try_commit_answer_verifier_retry_answer;

pub(crate) async fn retry_loop_answer_after_verifier(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    journal: &crate::task_journal::TaskJournal,
    rejected_answer: &str,
    verifier: &crate::answer_verifier::AnswerVerifierOut,
) -> Option<String> {
    retry_answer_after_verifier(
        state,
        task,
        user_request,
        journal,
        rejected_answer,
        verifier,
    )
    .await
}

#[cfg(test)]
use task_resume::{
    answer_verifier_retry_observed_trace, resume_context_has_directory_lookup_failure,
    resume_context_path_batch_facts_are_missing_only,
};

fn record_answer_verifier_required_evidence_rollout_attribution(
    journal: &mut crate::task_journal::TaskJournal,
) {
    let rollout_attribution =
        crate::task_journal::TaskJournalRolloutAttribution::answer_verifier_required_evidence_block(
            journal.answer_verifier_summary.as_ref(),
        );
    journal.record_rollout_attribution(rollout_attribution);
}

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

fn deterministic_filtered_log_entry_recovery(
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

fn content_tail_read_route_allows_failure_recovery(route_result: &crate::RouteResult) -> bool {
    let contract = route_result.effective_output_contract();
    !matches!(
        contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) && contract.requires_content_evidence
        && !contract.delivery_required
        && route_result.output_contract_marker_is_any(&[
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

fn deterministic_content_tail_read_failure_recovery(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    route_result: &crate::RouteResult,
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

fn raw_tail_read_route_allows_failure_recovery(route_result: &crate::RouteResult) -> bool {
    let contract = route_result.effective_output_contract();
    route_result.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
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

fn deterministic_raw_tail_read_failure_recovery(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    route_result: &crate::RouteResult,
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

fn assistant_memory_source_text(answer_text: &str, answer_messages: &[String]) -> String {
    let publishable_messages = answer_messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .collect::<Vec<_>>();
    if publishable_messages.is_empty() {
        let answer = answer_text.trim();
        if crate::finalize::is_execution_summary_message(answer) {
            String::new()
        } else {
            answer.to_string()
        }
    } else {
        publishable_messages.join("\n")
    }
}

fn should_reinsert_execution_summaries_for_delivery(
    route_result: &crate::RouteResult,
    answer_text: &str,
) -> bool {
    let output_contract = route_result.effective_output_contract();
    if (output_contract.response_shape == crate::OutputResponseShape::Scalar
        || route_requests_config_validation(route_result))
        && !answer_text.trim().is_empty()
        && !crate::finalize::is_execution_summary_message(answer_text)
    {
        return false;
    }
    if strict_structured_final_answer_suppresses_execution_summary(route_result, answer_text) {
        return false;
    }
    true
}

fn route_requests_config_validation(route_result: &crate::RouteResult) -> bool {
    crate::finalize::route_matches_validation_verdict_output_contract(route_result)
}

fn drop_execution_summaries_when_delivery_is_scalar(
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &mut Vec<String>,
) {
    if should_reinsert_execution_summaries_for_delivery(route_result, answer_text) {
        return;
    }
    answer_messages.retain(|message| !crate::finalize::is_execution_summary_message(message));
}

fn strict_structured_final_answer_suppresses_execution_summary(
    route_result: &crate::RouteResult,
    answer_text: &str,
) -> bool {
    route_result.output_contract.response_shape == crate::OutputResponseShape::Strict
        && !route_has_file_delivery_contract(route_result)
        && structured_machine_final_answer(answer_text)
}

fn structured_machine_final_answer(answer_text: &str) -> bool {
    let trimmed = answer_text.trim();
    if trimmed.is_empty() || crate::finalize::is_execution_summary_message(trimmed) {
        return false;
    }
    if serde_json::from_str::<Value>(trimmed)
        .ok()
        .is_some_and(|value| value.is_object() || value.is_array())
    {
        return true;
    }
    let lines = trimmed
        .split_whitespace()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    !lines.is_empty()
        && lines.iter().all(|line| {
            let Some((key, value)) = line.split_once('=') else {
                return false;
            };
            !key.trim().is_empty()
                && !value.trim().is_empty()
                && key
                    .trim()
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        })
}

fn journal_has_missing_file_search_evidence(
    journal: Option<&crate::task_journal::TaskJournal>,
) -> bool {
    journal
        .into_iter()
        .flat_map(|journal| journal.step_results.iter().rev())
        .any(|step| {
            step.output_excerpt
                .as_deref()
                .is_some_and(crate::finalize::loop_reply::output_excerpt_has_missing_file_evidence)
                || step
                    .error_excerpt
                    .as_deref()
                    .is_some_and(text_looks_like_missing_file_target)
        })
}

fn journal_has_non_control_step(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.step_results.iter().any(|step| {
        !matches!(
            step.skill.as_str(),
            "respond" | "synthesize_answer" | "think" | "answer_verifier"
        )
    })
}

fn delivery_path_gap_should_finalize_as_clarify(
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &[String],
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    route_has_file_delivery_contract(route_result)
        && !has_any_delivery_file_token(answer_text, answer_messages)
        && !journal_has_non_control_step(journal)
        && journal
            .answer_verifier_summary
            .as_ref()
            .is_some_and(|summary| {
                !summary.pass
                    && summary
                        .missing_evidence_fields
                        .iter()
                        .any(|field| field.trim() == "path")
            })
}

fn should_use_missing_file_delivery_reply(
    route_result: &crate::RouteResult,
    answer: &crate::AskReply,
) -> bool {
    route_has_file_delivery_contract(route_result)
        && !answer.should_fail_task
        && !has_any_delivery_file_token(&answer.text, &answer.messages)
        && journal_has_missing_file_search_evidence(answer.task_journal.as_ref())
}

async fn missing_file_delivery_reply_text(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    route_result: &crate::RouteResult,
    answer: &crate::AskReply,
) -> Option<String> {
    if !should_use_missing_file_delivery_reply(route_result, answer) {
        return None;
    }
    let language_hint = crate::language_policy::first_clear_request_language_hint([
        prompt,
        route_result.resolved_intent.as_str(),
    ])
    .unwrap_or_else(|| crate::language_policy::task_response_language_hint(state, task, prompt));
    Some(
        crate::fallback::compose_missing_file_delivery_response(
            state,
            task,
            prompt,
            &route_result.resolved_intent,
            Some(route_result.output_contract.locator_hint.as_str()),
            &language_hint,
        )
        .await,
    )
}

fn spawn_memory_intent_llm_extraction(state: &AppState, task: &crate::ClaimedTask, prompt: &str) {
    let state = state.clone();
    let mut task = task.clone();
    let parent_task_id = task.task_id.clone();
    task.task_id = format!("{parent_task_id}:memory_intent");
    let metrics_task_id = task.task_id.clone();
    let prompt = prompt.to_string();
    tokio::spawn(async move {
        if let Err(err) =
            crate::memory::maybe_extract_memory_intent_with_llm(&state, &task, &prompt).await
        {
            warn!(
                "memory intent llm extraction failed task_id={} parent_task_id={} err={}",
                task.task_id, parent_task_id, err
            );
        }
        state.clear_task_llm_call_count(&metrics_task_id);
    });
}

fn insert_ask_memory_pair(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    answer_text: &str,
    answer_messages: &[String],
    is_llm_reply: bool,
    agent_display_name_hint: &str,
) {
    let _ = crate::memory::upsert_user_preferences_from_route_hint(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        agent_display_name_hint,
    );
    spawn_memory_intent_llm_extraction(state, task, prompt);
    if should_skip_ask_memory_pair(state, answer_text, answer_messages) {
        return;
    }
    let _ = crate::memory::service::insert_memory(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &task.channel,
        task.external_chat_id.as_deref(),
        crate::memory::MEMORY_ROLE_USER,
        prompt,
        state.policy.memory.item_max_chars.max(256),
    );
    let assistant_source_text = assistant_memory_source_text(answer_text, answer_messages);
    if assistant_source_text.trim().is_empty() {
        return;
    }
    let assistant_memory_text = if is_llm_reply && state.policy.memory.mark_llm_reply_in_short_term
    {
        format!(
            "{}{}",
            crate::memory::LLM_SHORT_TERM_MEMORY_PREFIX,
            assistant_source_text
        )
    } else {
        assistant_source_text
    };
    let _ = crate::memory::service::insert_memory_with_kind(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &task.channel,
        task.external_chat_id.as_deref(),
        crate::memory::MEMORY_ROLE_ASSISTANT,
        &assistant_memory_text,
        state.policy.memory.item_max_chars.max(256),
        crate::memory::MemoryWriteKind::AssistantOutcome,
    );
}

fn build_unfinished_goal_memory_text(prompt: &str, blocker: &str) -> String {
    format!(
        "Unfinished goal\nUser request: {}\nCurrent blocker: {}",
        prompt.trim(),
        blocker.trim()
    )
}

fn insert_unfinished_goal_memory(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    blocker: &str,
) {
    let text = build_unfinished_goal_memory_text(prompt, blocker);
    let _ = crate::memory::service::insert_memory_with_kind(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &task.channel,
        task.external_chat_id.as_deref(),
        crate::memory::MEMORY_ROLE_SYSTEM,
        &text,
        state.policy.memory.item_max_chars.max(256),
        crate::memory::MemoryWriteKind::UnfinishedGoal,
    );
}

async fn finalize_ask_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    answer_text: &str,
    answer_messages: &[String],
    is_llm_reply: bool,
    should_refresh_long_term_memory: bool,
    agent_display_name_hint: &str,
    journal: &mut crate::task_journal::TaskJournal,
) -> Result<()> {
    let notify_outcome =
        crate::worker::maybe_notify_schedule_result(state, task, payload, true, answer_text).await;
    crate::worker::record_schedule_notify_outcome(journal, notify_outcome);
    let result = ask_result_payload(answer_text, answer_messages, Some(journal));
    repo::update_task_success(state, &task.task_id, &result.to_string())?;
    insert_ask_memory_pair(
        state,
        task,
        prompt,
        answer_text,
        answer_messages,
        is_llm_reply,
        agent_display_name_hint,
    );
    crate::worker::spawn_long_term_summary_refresh(
        state.clone(),
        task.clone(),
        should_refresh_long_term_memory,
    );
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=ask status=success path=normal result={}",
        task.task_id,
        crate::truncate_for_log(answer_text)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
}

fn journal_has_checkpointed_nonterminal_lifecycle(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let Some(lifecycle) = journal.task_lifecycle.as_ref() else {
        return false;
    };
    let state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(state, "waiting" | "background" | "needs_user") {
        return false;
    }
    let lifecycle_checkpoint_id = lifecycle
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let checkpoint_checkpoint_id = journal
        .task_checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    matches!(
        (lifecycle_checkpoint_id, checkpoint_checkpoint_id),
        (Some(lifecycle_id), Some(checkpoint_id)) if lifecycle_id == checkpoint_id
    )
}

async fn finalize_ask_checkpointed(
    state: &AppState,
    task: &crate::ClaimedTask,
    answer_text: &str,
    answer_messages: &[String],
    journal: &mut crate::task_journal::TaskJournal,
) -> Result<()> {
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    let result = ask_result_payload(answer_text, answer_messages, Some(journal));
    repo::update_task_progress_result(state, &task.task_id, &result.to_string())?;
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_checkpointed task_id={} kind=ask lifecycle_state={} checkpoint_id={}",
        task.task_id,
        journal
            .task_lifecycle
            .as_ref()
            .and_then(|lifecycle| lifecycle.get("state"))
            .and_then(|value| value.as_str())
            .unwrap_or("unknown"),
        journal
            .task_lifecycle
            .as_ref()
            .and_then(|lifecycle| lifecycle.get("checkpoint_id"))
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
}

async fn finalize_ask_resume_failure(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    user_error: &str,
    resume_payload: Value,
    answer_messages: &[String],
    journal: &mut crate::task_journal::TaskJournal,
) -> Result<()> {
    journal.record_final_failure_attribution_from_error(user_error);
    let notify_outcome =
        crate::worker::maybe_notify_schedule_result(state, task, payload, false, user_error).await;
    crate::worker::record_schedule_notify_outcome(journal, notify_outcome);
    let mut result = ask_result_payload(user_error, answer_messages, None);
    if let Some(obj) = result.as_object_mut() {
        obj.insert("resume_context".to_string(), resume_payload);
        obj.insert(
            "task_lifecycle".to_string(),
            failed_task_lifecycle_payload(user_error),
        );
    }
    let result = journal.attach_to_result(result);
    repo::update_task_failure_with_result(state, &task.task_id, &result.to_string(), user_error)?;
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=ask status=failed path=normal error={} resume_context=true",
        task.task_id,
        crate::truncate_for_log(user_error)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
}

async fn finalize_ask_failure(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    answer_text: &str,
    answer_messages: &[String],
    err_text: &str,
    journal: &mut crate::task_journal::TaskJournal,
) -> Result<()> {
    error!(
        "worker_once: ask task_id={} failed: {}",
        task.task_id, err_text
    );
    journal.record_final_failure_attribution_from_error(err_text);
    let notify_outcome =
        crate::worker::maybe_notify_schedule_result(state, task, payload, false, answer_text).await;
    crate::worker::record_schedule_notify_outcome(journal, notify_outcome);
    let mut result = ask_result_payload(answer_text, answer_messages, None);
    if let Some(obj) = result.as_object_mut() {
        obj.insert(
            "task_lifecycle".to_string(),
            failed_task_lifecycle_payload(err_text),
        );
    }
    let result = journal.attach_to_result(result);
    repo::update_task_failure_with_result(state, &task.task_id, &result.to_string(), err_text)?;
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=ask status=failed path=normal error={}",
        task.task_id,
        crate::truncate_for_log(err_text)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
}

async fn compose_ask_failure_user_message(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    err_text: &str,
) -> String {
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let fallback_payload = ask_runtime_failure_machine_payload(err_text);
    let observed_facts = machine_payload_observed_facts(&fallback_payload);
    let contract = crate::fallback::UserResponseContract::tool_failure(
        "ask_runtime_failure",
        user_request,
        user_request,
        observed_facts,
        vec![
            "expose_internal_details=false".to_string(),
            "task_success_claim_allowed=false".to_string(),
            "unobserved_action_completion_claim_allowed=false".to_string(),
            "recovery_path_policy=one_concise_actionable".to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    let default_text = ask_runtime_failure_default_text(err_text);
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &default_text,
    )
    .await
}

fn ask_runtime_failure_default_text(err: &str) -> String {
    ask_runtime_failure_machine_payload(err)
}

pub(crate) async fn finalize_ask_direct_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    answer_text: &str,
    path_label: &str,
    should_refresh_long_term_memory: bool,
    agent_display_name_hint: &str,
) -> Result<()> {
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", prompt);
    journal.record_context_bundle_summary(format!("path={path_label}"));
    journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
    journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
    journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
    journal.record_used_evidence_ids_count(0);
    journal.record_delivery_consistent(crate::task_journal::delivery_payload_consistent(
        answer_text,
        &[],
    ));
    journal.record_final_answer(answer_text);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    let notify_outcome =
        crate::worker::maybe_notify_schedule_result(state, task, payload, true, answer_text).await;
    crate::worker::record_schedule_notify_outcome(&mut journal, notify_outcome);
    let result = journal.attach_to_result(json!({ "text": answer_text }));
    repo::update_task_success(state, &task.task_id, &result.to_string())?;
    insert_ask_memory_pair(
        state,
        task,
        prompt,
        answer_text,
        &[],
        false,
        agent_display_name_hint,
    );
    crate::worker::spawn_long_term_summary_refresh(
        state.clone(),
        task.clone(),
        should_refresh_long_term_memory,
    );
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=ask status=success path={} result={}",
        task.task_id,
        path_label,
        crate::truncate_for_log(answer_text)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    state.clear_task_llm_call_count(&task.task_id);
    Ok(())
}

pub(crate) async fn run_direct_classifier_reply(
    state: &AppState,
    task: &crate::ClaimedTask,
    resolved_prompt_for_execution: &str,
) -> Result<crate::AskReply, String> {
    const DIRECT_CLASSIFIER_PROMPT_LABEL: &str = "inline:direct_classifier";
    let request_language_hint = crate::language_policy::task_response_language_hint(
        state,
        task,
        resolved_prompt_for_execution,
    );
    let prompt = format!(
        "You are producing the final user-facing reply directly.\n\nRequest language hint: {request_language_hint}\nConfigured fallback language: {}\n\nLanguage policy (strict): follow the Request language hint when it is clear. Clear hints include `zh-CN`, `en`, `mixed`, BCP-47 style language tags such as `ja`/`ko`/`fr-FR`, and script hints such as `und-Latn`/`und-Cyrl`/`und-Arab`. Use the configured fallback language only when the hint is `config_default` or otherwise unclear. If the hint is `en` but the current request is clearly another Latin-script human language, follow the current request language. Do not switch languages just because names, paths, commands, code, or other normalized values are in English.\n\nReturn only the user-facing reply.\n\n{}",
        state.policy.command_intent.default_locale,
        resolved_prompt_for_execution
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "direct_classifier",
        DIRECT_CLASSIFIER_PROMPT_LABEL,
        None,
    );
    crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        DIRECT_CLASSIFIER_PROMPT_LABEL,
    )
    .await
    .map(|s| crate::AskReply::llm(s.trim().to_string()))
    .map_err(|e| e.to_string())
}

pub(crate) async fn try_finalize_schedule_direct_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    _resolved_prompt_for_execution: &str,
    route_result: &crate::RouteResult,
) -> Result<bool> {
    if let Ok(Some(schedule_reply)) = crate::intent_router::try_handle_schedule_request(
        state,
        task,
        prompt,
        route_result.schedule_intent.as_ref(),
    )
    .await
    {
        let schedule_reply = crate::intercept_response_text_for_delivery(&schedule_reply);
        finalize_ask_direct_success(
            state,
            task,
            payload,
            prompt,
            &schedule_reply,
            "schedule_direct",
            route_result.should_refresh_long_term_memory,
            &route_result.agent_display_name_hint,
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}

fn should_use_answer_route_result(
    initial: &crate::RouteResult,
    answer_route: &crate::RouteResult,
    answer_journal: &crate::task_journal::TaskJournal,
) -> bool {
    let answer_is_clarify = answer_journal.final_status.is_some_and(|status| {
        matches!(status, crate::task_journal::TaskJournalFinalStatus::Clarify)
    });
    if answer_is_clarify && !initial.needs_clarify {
        return true;
    }
    let answer_has_execution_trace = !answer_journal.rounds.is_empty()
        || !answer_journal.step_results.is_empty()
        || answer_journal.plan_result.is_some()
        || answer_journal.verify_result.is_some();
    answer_has_execution_trace && answer_route.is_execute_gate() && !initial.is_execute_gate()
}

fn answer_verifier_recovery_already_terminal(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.final_status.is_some_and(|status| {
        matches!(status, crate::task_journal::TaskJournalFinalStatus::Success)
    }) && journal.answer_verifier_summary.is_none()
        && journal
            .final_stop_signal
            .as_deref()
            .is_some_and(crate::task_journal::is_answer_verifier_recovered_terminal_stop_signal)
}

fn turn_analysis_requires_machine_summary(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(state_patch) = turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()) else {
        return false;
    };
    state_patch.get("required_machine_fields").is_some()
        || state_patch
            .get("output_format")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|format| format == "machine_summary")
}

fn route_allows_verified_terminal_answer_promotion(
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    if route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || turn_analysis_requires_machine_summary(journal.turn_analysis.as_ref())
    {
        return false;
    }
    crate::evidence_policy::final_answer_shape_for_route(route_result)
        .is_some_and(|shape| shape.allows_model_language())
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        )
}

pub(super) fn promote_verified_terminal_answer_after_verifier_pass(
    route_result: &crate::RouteResult,
    journal: &mut crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    if answer_verifier_recovery_already_terminal(journal) {
        return false;
    }
    if !route_allows_verified_terminal_answer_promotion(route_result, journal) {
        return false;
    }
    let Some(recovered_answer) = verified_terminal_answer_after_verifier_pass(journal) else {
        return false;
    };
    if recovered_answer.trim() == answer_text.trim() {
        return false;
    }
    *answer_text = recovered_answer;
    answer_messages.clear();
    answer_messages.push(answer_text.clone());
    journal.record_final_answer(answer_text.as_str());
    true
}

pub(crate) async fn finalize_ask_result(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    context_bundle_summary: &str,
    memory_trace: Option<&Value>,
    resolved_prompt_for_execution: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    fuzzy_locator_suggestions: &[String],
    clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    result: Result<crate::AskReply, String>,
) -> Result<()> {
    // §3.1: ask 状态机 — 进入 finalize。
    // from = None 因为 dispatch 内部各分支态没向调用面回传"上一次状态"；
    // reason 携带 ask_mode 信息以便日志检索。
    let finalize_entry_transition = crate::log_ask_transition(
        state,
        &task.task_id,
        None,
        crate::AskState::Finalizing,
        &format!(
            "finalize_ask_result_entry mode={}",
            route_result.ask_mode.as_str()
        ),
        None,
    );
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", prompt);
    journal.transitions.push(finalize_entry_transition);
    if let Some(turn_analysis) = turn_analysis {
        journal.record_turn_analysis(turn_analysis);
    }
    journal.record_route_result(route_result);
    journal.record_context_bundle_summary(format!(
        "{} needs_clarify={} resolved_prompt={}",
        context_bundle_summary,
        route_result.ask_mode.is_clarify_only(),
        crate::truncate_for_log(resolved_prompt_for_execution)
    ));
    if let Some(memory_trace) = memory_trace {
        journal.record_memory_trace(memory_trace.clone());
    }
    match result {
        Ok(answer) => {
            if !repo::is_task_still_running_or_pending_ask_success_projection(state, &task.task_id)?
            {
                state.clear_task_llm_call_count(&task.task_id);
                info!(
                    "task_call_end task_id={} kind=ask status=canceled path=normal",
                    task.task_id
                );
                return Ok(());
            }
            let mut effective_route_result = route_result.clone();
            if let Some(answer_journal) = answer.task_journal.as_ref() {
                journal.merge_from(answer_journal);
                if let Some(answer_route_result) = answer_journal.route_result.as_ref() {
                    if should_use_answer_route_result(
                        route_result,
                        answer_route_result,
                        answer_journal,
                    ) {
                        effective_route_result = answer_route_result.clone();
                        journal.record_route_result(&effective_route_result);
                    }
                }
            }
            let route_result = &effective_route_result;
            let mut semantic_clarify = route_result.ask_mode.is_clarify_only()
                || answer
                    .task_journal
                    .as_ref()
                    .and_then(|journal| journal.final_status)
                    .is_some_and(|status| {
                        matches!(status, crate::task_journal::TaskJournalFinalStatus::Clarify)
                    });
            let mut failure_reply = answer.should_fail_task;
            let missing_file_delivery_reply =
                missing_file_delivery_reply_text(state, task, prompt, route_result, &answer).await;
            let (mut answer_text, mut answer_messages) = if failure_reply
                || route_result.ask_mode.is_clarify_only()
            {
                (
                    crate::intercept_response_text_for_delivery(&answer.text),
                    answer
                        .messages
                        .into_iter()
                        .map(|message| message.trim().to_string())
                        .filter(|message| !message.is_empty())
                        .collect(),
                )
            } else if let Some(reply_text) = missing_file_delivery_reply {
                (reply_text.clone(), vec![reply_text])
            } else {
                let original_messages = answer.messages;
                let execution_summaries = original_messages
                    .iter()
                    .map(|message| message.trim().to_string())
                    .filter(|message| !message.is_empty())
                    .filter(|message| crate::finalize::is_execution_summary_message(message))
                    .collect::<Vec<_>>();
                let (answer_text, mut answer_messages) =
                    crate::intercept_response_payload_for_delivery(
                        state,
                        // Delivery interception must stay grounded in the original user request.
                        // The execution prompt may contain injected runtime hints such as
                        // [AUTO_LOCATOR], which are useful for planning/execution but must not be
                        // reinterpreted as fresh user-provided locator input during final delivery
                        // normalization.
                        prompt,
                        route_result.wants_file_delivery,
                        &route_result.output_contract,
                        answer.text,
                        original_messages,
                    );
                if should_reinsert_execution_summaries_for_delivery(route_result, &answer_text) {
                    for summary in execution_summaries.into_iter().rev() {
                        if !answer_messages.iter().any(|message| message == &summary) {
                            answer_messages.insert(0, summary);
                        }
                    }
                } else {
                    drop_execution_summaries_when_delivery_is_scalar(
                        route_result,
                        &answer_text,
                        &mut answer_messages,
                    );
                }
                (answer_text, answer_messages)
            };
            backfill_file_delivery_contract_from_journal(
                route_result,
                &journal,
                &mut answer_text,
                &mut answer_messages,
            );
            journal.record_final_answer(&answer_text);
            if task_terminal_clarify::preserve_terminal_clarify_from_journal(
                &journal,
                &mut answer_text,
                &mut answer_messages,
            ) {
                failure_reply = false;
                semantic_clarify = true;
                journal.answer_verifier_summary = None;
                journal.record_final_answer(&answer_text);
            }
            if recover_raw_command_machine_field_final_answer(
                route_result,
                &mut journal,
                &mut answer_text,
                &mut answer_messages,
            ) {
                failure_reply = false;
                semantic_clarify = false;
                info!(
                    "finalize_raw_command_machine_fields_recovered task_id={} answer={}",
                    task.task_id,
                    crate::truncate_for_log(&answer_text)
                );
            }
            if failure_reply {
                if let Some(recovered_answer) = deterministic_raw_tail_read_failure_recovery(
                    state,
                    task,
                    prompt,
                    route_result,
                    &journal,
                ) {
                    failure_reply = false;
                    semantic_clarify = false;
                    answer_text = recovered_answer;
                    answer_messages.clear();
                    answer_messages.push(answer_text.clone());
                    journal.answer_verifier_summary = None;
                    journal.record_final_answer(&answer_text);
                    info!(
                        "finalize_raw_tail_read_failure_recovered task_id={} answer={}",
                        task.task_id,
                        crate::truncate_for_log(&answer_text)
                    );
                }
            }
            let answer_is_existing_file_delivery_token = if let Some(token) =
                normalize_existing_file_delivery_token_answer(state, &answer_text)
            {
                if answer_text.trim() != token {
                    answer_text = token;
                    answer_messages.clear();
                    answer_messages.push(answer_text.clone());
                    journal.record_final_answer(&answer_text);
                }
                true
            } else {
                false
            };
            if !failure_reply
                && !semantic_clarify
                && journal.answer_verifier_summary.is_none()
                && !answer_verifier_recovery_already_terminal(&journal)
            {
                let answer_verifier = if answer_is_existing_file_delivery_token {
                    None
                } else {
                    crate::answer_verifier::verify_answer_observe_only(
                        state,
                        task,
                        prompt,
                        route_result,
                        &journal,
                        &answer_text,
                    )
                    .await
                };
                if let Some(answer_verifier) = answer_verifier {
                    let answer_verifier_retry =
                        answer_verifier_retry_applicable(route_result, &journal, &answer_verifier);
                    let retry_verifier_input = answer_verifier.clone();
                    journal.record_answer_verifier_summary(answer_verifier);
                    if answer_verifier_retry {
                        let recovered_by_machine_evidence =
                            recover_answer_verifier_gap_with_deterministic_machine_evidence(
                                prompt,
                                route_result,
                                &mut journal,
                                &mut answer_text,
                                &mut answer_messages,
                            );
                        if recovered_by_machine_evidence {
                            failure_reply = false;
                            semantic_clarify = false;
                            info!(
                                "finalize_answer_verifier_gap_recovered_before_llm_retry task_id={} answer={}",
                                task.task_id,
                                crate::truncate_for_log(&answer_text)
                            );
                        } else if let Some(retried_answer) = retry_answer_after_verifier(
                            state,
                            task,
                            prompt,
                            &journal,
                            &answer_text,
                            &retry_verifier_input,
                        )
                        .await
                        {
                            if try_commit_answer_verifier_retry_answer(
                                state,
                                task,
                                prompt,
                                route_result,
                                &mut journal,
                                &mut answer_text,
                                &mut answer_messages,
                                retried_answer,
                            )
                            .await
                            {
                                failure_reply = false;
                                semantic_clarify = false;
                            }
                        }
                    }
                }
            }
            if let Some(recovered_answer) = deterministic_structured_evidence_table_recovery(
                route_result,
                &journal,
                failure_reply,
            ) {
                failure_reply = false;
                semantic_clarify = false;
                answer_text = recovered_answer;
                answer_messages.clear();
                answer_messages.push(answer_text.clone());
                journal.record_final_answer(&answer_text);
                mark_answer_verifier_recovered_by_deterministic_observed_evidence(&mut journal);
                info!(
                    "finalize_structured_evidence_table_recovered task_id={} answer={}",
                    task.task_id,
                    crate::truncate_for_log(&answer_text)
                );
            }
            if let Some(recovered_answer) = deterministic_filtered_log_entry_recovery(&journal) {
                failure_reply = false;
                answer_text = recovered_answer;
                answer_messages.clear();
                answer_messages.push(answer_text.clone());
                journal.record_final_answer(&answer_text);
                mark_answer_verifier_recovered_by_deterministic_observed_evidence(&mut journal);
            }
            if let Some(recovered_answer) =
                deterministic_config_guard_candidates_recovery(route_result, &journal)
            {
                failure_reply = false;
                semantic_clarify = false;
                answer_text = recovered_answer;
                answer_messages.clear();
                answer_messages.push(answer_text.clone());
                journal.record_final_answer(&answer_text);
                mark_answer_verifier_recovered_by_deterministic_observed_evidence(&mut journal);
                info!(
                    "finalize_config_guard_candidates_recovered task_id={} answer={}",
                    task.task_id,
                    crate::truncate_for_log(&answer_text)
                );
            }
            if let Some(recovered_answer) = deterministic_content_tail_read_failure_recovery(
                state,
                task,
                prompt,
                route_result,
                &journal,
            ) {
                failure_reply = false;
                semantic_clarify = false;
                answer_text = recovered_answer;
                answer_messages.clear();
                answer_messages.push(answer_text.clone());
                journal.record_final_answer(&answer_text);
                mark_answer_verifier_recovered_by_deterministic_observed_evidence(&mut journal);
                info!(
                    "finalize_content_tail_read_failure_recovered task_id={} answer={}",
                    task.task_id,
                    crate::truncate_for_log(&answer_text)
                );
            }
            let mut recovered_structured_machine_rows = false;
            if let Some(recovered_answer) =
                deterministic_tree_summary_rows_failure_recovery(&journal)
            {
                failure_reply = false;
                semantic_clarify = false;
                recovered_structured_machine_rows = true;
                answer_text = recovered_answer;
                answer_messages.clear();
                answer_messages.push(answer_text.clone());
                journal.record_final_answer(&answer_text);
                mark_answer_verifier_recovered_by_deterministic_observed_evidence(&mut journal);
                info!(
                    "finalize_tree_summary_rows_failure_recovered task_id={} answer={}",
                    task.task_id,
                    crate::truncate_for_log(&answer_text)
                );
            }
            let mut recovered_requested_machine_kv_summary = false;
            let force_requested_machine_kv_summary =
                failure_reply || answer_verifier_forces_task_failure(semantic_clarify, &journal);
            if !semantic_clarify
                && !recovered_structured_machine_rows
                && recover_requested_machine_kv_summary_final_answer(
                    prompt,
                    route_result,
                    &mut journal,
                    &mut answer_text,
                    &mut answer_messages,
                    force_requested_machine_kv_summary,
                )
            {
                failure_reply = false;
                semantic_clarify = false;
                recovered_requested_machine_kv_summary = true;
                info!(
                    "finalize_requested_machine_kv_summary_recovered task_id={} answer={}",
                    task.task_id,
                    crate::truncate_for_log(&answer_text)
                );
            }
            if !failure_reply
                && !semantic_clarify
                && !recovered_structured_machine_rows
                && !recovered_requested_machine_kv_summary
                && recover_requested_machine_kv_summary_final_answer(
                    prompt,
                    route_result,
                    &mut journal,
                    &mut answer_text,
                    &mut answer_messages,
                    false,
                )
            {
                info!(
                    "finalize_requested_machine_kv_summary_recovered task_id={} answer={}",
                    task.task_id,
                    crate::truncate_for_log(&answer_text)
                );
            }
            if delivery_path_gap_should_finalize_as_clarify(
                route_result,
                &answer_text,
                &answer_messages,
                &journal,
            ) {
                semantic_clarify = true;
                journal.answer_verifier_summary = None;
            }
            if recover_raw_command_machine_field_final_answer(
                route_result,
                &mut journal,
                &mut answer_text,
                &mut answer_messages,
            ) {
                failure_reply = false;
                semantic_clarify = false;
                info!(
                    "finalize_raw_command_machine_fields_recovered task_id={} answer={}",
                    task.task_id,
                    crate::truncate_for_log(&answer_text)
                );
            }
            if !semantic_clarify
                && promote_verified_terminal_answer_after_verifier_pass(
                    route_result,
                    &mut journal,
                    &mut answer_text,
                    &mut answer_messages,
                )
            {
                failure_reply = false;
                info!(
                    "finalize_verified_terminal_answer_promoted task_id={} answer={}",
                    task.task_id,
                    crate::truncate_for_log(&answer_text)
                );
            }
            backfill_file_delivery_contract_from_journal(
                route_result,
                &journal,
                &mut answer_text,
                &mut answer_messages,
            );
            journal.record_final_answer(&answer_text);
            journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
            journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
            journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
            crate::finalize::ensure_task_metrics(&mut journal, &answer_text, &answer_messages);
            if failure_reply {
                if let Some(recovered_answer) =
                    verified_terminal_answer_after_verifier_pass(&journal)
                {
                    failure_reply = false;
                    semantic_clarify = false;
                    answer_text = recovered_answer;
                    answer_messages.clear();
                    answer_messages.push(answer_text.clone());
                    journal.record_final_answer(&answer_text);
                    info!(
                        "finalize_verified_terminal_answer_recovered task_id={} answer={}",
                        task.task_id,
                        crate::truncate_for_log(&answer_text)
                    );
                }
            }
            if !semantic_clarify
                && answer.resume_context.is_none()
                && journal_has_checkpointed_nonterminal_lifecycle(&journal)
            {
                finalize_ask_checkpointed(
                    state,
                    task,
                    &answer_text,
                    &answer_messages,
                    &mut journal,
                )
                .await?;
                return Ok(());
            }
            if failure_reply {
                let err_text = answer.error_text.unwrap_or_else(|| answer_text.clone());
                if let Some(resume_payload) = answer.resume_context {
                    journal.record_final_status(
                        crate::task_journal::TaskJournalFinalStatus::ResumeFailure,
                    );
                    finalize_ask_resume_failure(
                        state,
                        task,
                        payload,
                        &err_text,
                        resume_payload,
                        &answer_messages,
                        &mut journal,
                    )
                    .await?;
                    insert_unfinished_goal_memory(state, task, prompt, &err_text);
                } else {
                    journal
                        .record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
                    let (visible_failure_text, visible_failure_messages) =
                        if answer_verifier_failure_needs_user_message(&answer_text, &err_text) {
                            let visible = compose_answer_verifier_failure_user_message(
                                state, task, prompt, &err_text,
                            );
                            (visible.clone(), vec![visible])
                        } else {
                            (answer_text.clone(), answer_messages.clone())
                        };
                    journal.record_final_answer(&visible_failure_text);
                    finalize_ask_failure(
                        state,
                        task,
                        payload,
                        &visible_failure_text,
                        &visible_failure_messages,
                        &err_text,
                        &mut journal,
                    )
                    .await?;
                    insert_unfinished_goal_memory(state, task, prompt, &err_text);
                }
            } else if answer_verifier_should_force_task_failure(
                crate::agent_engine::answer_verifier_enforce_required_enabled_for_route(
                    state,
                    Some(route_result),
                ),
                semantic_clarify,
                &journal,
            ) {
                record_answer_verifier_required_evidence_rollout_attribution(&mut journal);
                let err_text = journal
                    .answer_verifier_summary
                    .as_ref()
                    .map(|summary| summary.required_evidence_failure_payload_text())
                    .unwrap_or_else(|| {
                        json!({
                            "schema_version": 1,
                            "message_key": "answer_verifier_required_evidence_block",
                            "reason_code": "answer_verifier_required_evidence_block",
                            "status_code": "answer_verifier_required_evidence_block",
                            "failure_attribution": "answer_verifier_gap",
                            "retryable": false,
                        })
                        .to_string()
                    });
                journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
                let (visible_failure_text, visible_failure_messages) =
                    if answer_verifier_failure_needs_user_message(&answer_text, &err_text) {
                        let visible = compose_answer_verifier_failure_user_message(
                            state, task, prompt, &err_text,
                        );
                        (visible.clone(), vec![visible])
                    } else {
                        (answer_text.clone(), answer_messages.clone())
                    };
                journal.record_final_answer(&visible_failure_text);
                finalize_ask_failure(
                    state,
                    task,
                    payload,
                    &visible_failure_text,
                    &visible_failure_messages,
                    &err_text,
                    &mut journal,
                )
                .await?;
            } else {
                journal.record_final_status(non_failure_final_status(semantic_clarify));
                finalize_ask_success(
                    state,
                    task,
                    payload,
                    prompt,
                    &answer_text,
                    &answer_messages,
                    answer.is_llm_reply,
                    route_result.should_refresh_long_term_memory,
                    &route_result.agent_display_name_hint,
                    &mut journal,
                )
                .await?;
                crate::conversation_state::update_active_session_from_ask_outcome(
                    state,
                    task,
                    Some(payload),
                    prompt,
                    route_result,
                    turn_analysis,
                    resolved_prompt_for_execution,
                    &answer_text,
                    &answer_messages,
                    semantic_clarify,
                    fuzzy_locator_suggestions,
                    &journal,
                    clarify_fallback_source,
                );
                if semantic_clarify {
                    insert_unfinished_goal_memory(state, task, prompt, &answer_text);
                }
            }
            // §3.1: Finalizing → Completed（成功路径，含 success / failure / resume_failure / clarify
            // 子分类，在 final_status 字段已区分；这里 ask 状态机视为正常完成 = Completed）。
            // 真实失败的 Err(...) 入分支会在下方打 Failed。
            let completed_transition = crate::log_ask_transition(
                state,
                &task.task_id,
                Some(crate::AskState::Finalizing),
                crate::AskState::Completed,
                "finalize_ok",
                None,
            );
            journal.transitions.push(completed_transition);
            info!(
                "task_journal_summary task_id={} kind=ask phase=finalize {}",
                task.task_id,
                journal.to_log_json()
            );
            state.clear_task_llm_call_count(&task.task_id);
        }
        Err(err_text) => {
            if err_text == crate::agent_engine::TASK_CANCELED_ERR
                || !repo::is_task_still_running(state, &task.task_id)?
            {
                state.clear_task_llm_call_count(&task.task_id);
                info!(
                    "task_call_end task_id={} kind=ask status=canceled path=normal",
                    task.task_id
                );
                return Ok(());
            }
            if let Some((user_error, resume_ctx)) = crate::parse_resume_context_error(&err_text) {
                info!(
                    "task_journal_summary task_id={} kind=ask phase=resume_failure {}",
                    task.task_id,
                    journal.to_log_json()
                );
                let resume_payload = resume_ctx
                    .get("resume_context")
                    .cloned()
                    .unwrap_or(resume_ctx);
                let language_hint =
                    crate::language_policy::task_response_language_hint(state, task, prompt);
                let prefer_english = language_hint.to_ascii_lowercase().starts_with("en");
                if resume_failure_is_unbound_path_lookup_clarify_result(
                    route_result,
                    &resume_payload,
                ) {
                    journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
                    journal
                        .record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
                    journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
                    journal.record_final_answer(&user_error);
                    crate::finalize::ensure_task_metrics(&mut journal, &user_error, &[]);
                    journal
                        .record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);
                    finalize_ask_success(
                        state,
                        task,
                        payload,
                        prompt,
                        &user_error,
                        &[],
                        false,
                        route_result.should_refresh_long_term_memory,
                        &route_result.agent_display_name_hint,
                        &mut journal,
                    )
                    .await?;
                    crate::conversation_state::update_active_session_from_ask_outcome(
                        state,
                        task,
                        Some(payload),
                        prompt,
                        route_result,
                        turn_analysis,
                        resolved_prompt_for_execution,
                        &user_error,
                        &[],
                        true,
                        fuzzy_locator_suggestions,
                        &journal,
                        clarify_fallback_source,
                    );
                    insert_unfinished_goal_memory(state, task, prompt, &user_error);
                    let completed_transition = crate::log_ask_transition(
                        state,
                        &task.task_id,
                        Some(crate::AskState::Finalizing),
                        crate::AskState::Completed,
                        "finalize_unbound_path_lookup_resume_clarify",
                        None,
                    );
                    journal.transitions.push(completed_transition);
                    info!(
                        "task_journal_summary task_id={} kind=ask phase=resume_clarify reason=unbound_path_lookup {}",
                        task.task_id,
                        journal.to_log_json()
                    );
                    state.clear_task_llm_call_count(&task.task_id);
                    return Ok(());
                }
                let qualified_resume_completion = if resume_failure_is_missing_file_delivery_result(
                    route_result,
                    &resume_payload,
                ) {
                    Some(("missing_file_delivery", user_error.clone()))
                } else if resume_failure_is_structured_service_status_result(
                    route_result,
                    &resume_payload,
                ) {
                    Some(("structured_service_status", user_error.clone()))
                } else if let Some(answer) = resume_failure_execution_failed_step_answer(
                    route_result,
                    &resume_payload,
                    prefer_english,
                ) {
                    Some(("execution_failed_step", answer))
                } else {
                    None
                };
                if let Some((qualified_resume_reason, qualified_answer)) =
                    qualified_resume_completion
                {
                    let mut answer_messages =
                        resume_context_execution_summary_messages(&resume_payload, prefer_english);
                    answer_messages.push(qualified_answer.clone());
                    journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
                    journal
                        .record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
                    journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
                    journal.record_final_answer(&qualified_answer);
                    crate::finalize::ensure_task_metrics(
                        &mut journal,
                        &qualified_answer,
                        &answer_messages,
                    );
                    journal
                        .record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
                    finalize_ask_success(
                        state,
                        task,
                        payload,
                        prompt,
                        &qualified_answer,
                        &answer_messages,
                        false,
                        route_result.should_refresh_long_term_memory,
                        &route_result.agent_display_name_hint,
                        &mut journal,
                    )
                    .await?;
                    crate::conversation_state::update_active_session_from_ask_outcome(
                        state,
                        task,
                        Some(payload),
                        prompt,
                        route_result,
                        turn_analysis,
                        resolved_prompt_for_execution,
                        &qualified_answer,
                        &answer_messages,
                        false,
                        fuzzy_locator_suggestions,
                        &journal,
                        clarify_fallback_source,
                    );
                    let completed_transition = crate::log_ask_transition(
                        state,
                        &task.task_id,
                        Some(crate::AskState::Finalizing),
                        crate::AskState::Completed,
                        &format!("finalize_{qualified_resume_reason}_resume_success"),
                        None,
                    );
                    journal.transitions.push(completed_transition);
                    info!(
                        "task_journal_summary task_id={} kind=ask phase=qualified_resume_success reason={} {}",
                        task.task_id,
                        qualified_resume_reason,
                        journal.to_log_json()
                    );
                    state.clear_task_llm_call_count(&task.task_id);
                    return Ok(());
                }
                journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
                journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
                journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
                journal.record_final_answer(&user_error);
                crate::finalize::ensure_task_metrics(&mut journal, &user_error, &[]);
                journal.record_final_status(
                    crate::task_journal::TaskJournalFinalStatus::ResumeFailure,
                );
                finalize_ask_resume_failure(
                    state,
                    task,
                    payload,
                    &user_error,
                    resume_payload,
                    &[],
                    &mut journal,
                )
                .await?;
                insert_unfinished_goal_memory(state, task, prompt, &user_error);
                // §3.1: Finalizing → Failed (resume_failure 子路径)。
                crate::log_ask_transition(
                    state,
                    &task.task_id,
                    Some(crate::AskState::Finalizing),
                    crate::AskState::Failed,
                    "finalize_resume_failure",
                    None,
                );
                state.clear_task_llm_call_count(&task.task_id);
                return Ok(());
            }
            let user_error = compose_ask_failure_user_message(state, task, prompt, &err_text).await;
            journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
            journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
            journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
            journal.record_final_answer(&user_error);
            crate::finalize::ensure_task_metrics(&mut journal, &user_error, &[]);
            journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
            finalize_ask_failure(
                state,
                task,
                payload,
                &user_error,
                &[],
                &err_text,
                &mut journal,
            )
            .await?;
            insert_unfinished_goal_memory(state, task, prompt, &user_error);
            // §3.1: Finalizing → Failed（dispatch 抛 Err 进入此分支）。
            crate::log_ask_transition(
                state,
                &task.task_id,
                Some(crate::AskState::Finalizing),
                crate::AskState::Failed,
                "finalize_err",
                None,
            );
            info!(
                "task_journal_summary task_id={} kind=ask phase=failure error={} {}",
                task.task_id,
                crate::truncate_for_log(&err_text),
                journal.to_log_json()
            );
            state.clear_task_llm_call_count(&task.task_id);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;

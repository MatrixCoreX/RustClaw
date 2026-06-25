use tracing::info;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{log_deterministic_delivery_record, looks_like_structured_machine_output};

pub(super) fn latest_plan_requested_synthesis(loop_state: &LoopState) -> bool {
    loop_state.round_traces.iter().rev().any(|round| {
        round
            .plan_result
            .as_ref()
            .is_some_and(|plan| plan.raw_plan_text.contains("\"synthesize_answer\""))
    })
}

pub(super) fn latest_path_batch_facts_has_implicit_metadata_fields(loop_state: &LoopState) -> bool {
    let Some(observed) =
        crate::agent_engine::observed_output::extract_latest_generic_successful_output(loop_state)
    else {
        return false;
    };
    if !matches!(observed.skill.as_str(), "fs_basic" | "system_basic") {
        return false;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&observed.body) else {
        return false;
    };
    if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts")
        || value.get("fields").is_some()
    {
        return false;
    }
    value
        .get("facts")
        .and_then(|value| value.as_array())
        .is_some_and(|facts| {
            facts.iter().any(|entry| {
                entry
                    .get("fact")
                    .and_then(|value| value.as_object())
                    .is_some_and(|fact| {
                        fact.get("size_bytes").is_some() || fact.get("modified_ts").is_some()
                    })
            })
        })
}

pub(super) fn route_allows_latest_tail_read_range_delivery(route: &crate::RouteResult) -> bool {
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) {
        return false;
    }
    !route.output_contract.delivery_required
        && route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ExcerptKindJudgment
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::None
        )
}

pub(super) fn latest_tail_read_range_observed_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_allows_latest_tail_read_range_delivery(route) {
        return None;
    }
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let prefer_english =
        crate::fallback::fallback_prefers_english_for_language_hint(state, &language_hint);
    let answer = if route_prefers_deterministic_tail_line(Some(route)) {
        latest_tail_read_range_selected_line_from_loop(loop_state, prefer_english)
            .or_else(|| latest_tail_read_range_answer_from_loop(loop_state, prefer_english))?
    } else {
        latest_tail_read_range_answer_from_loop(loop_state, prefer_english)?
    };
    if answer.trim().is_empty() {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

pub(super) fn latest_tail_read_range_answer_from_loop(
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?.trim();
            let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
            tail_read_range_answer_from_value(&value, prefer_english)
        })
}

fn step_output_is_tail_read_range(step: &crate::executor::StepExecutionResult) -> bool {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
        return false;
    }
    let Some(output) = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
        return false;
    };
    value_is_tail_read_range(&value)
}

fn value_is_tail_read_range(value: &serde_json::Value) -> bool {
    if flat_value_is_tail_read_range(value) {
        return true;
    }
    value.get("extra").is_some_and(value_is_tail_read_range)
        || value
            .get("text")
            .and_then(|text| text.as_str())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            .as_ref()
            .is_some_and(value_is_tail_read_range)
}

fn flat_value_is_tail_read_range(value: &serde_json::Value) -> bool {
    matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("read_range" | "read_text_range")
    ) && value.get("mode").and_then(|value| value.as_str()) == Some("tail")
        && tail_read_requested_n(value)
            .is_some_and(|requested_n| requested_n > 0 && requested_n <= 50)
        && value
            .get("excerpt")
            .and_then(|value| value.as_str())
            .is_some_and(|excerpt| !excerpt.trim().is_empty())
}

fn tail_read_requested_n(value: &serde_json::Value) -> Option<u64> {
    value
        .get("requested_n")
        .or_else(|| value.get("n"))
        .or_else(|| value.get("count"))
        .and_then(|value| value.as_u64())
}

fn tail_read_range_answer_from_value(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if let Some(answer) = flat_tail_read_range_answer_from_value(value, prefer_english) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(|extra| tail_read_range_answer_from_value(extra, prefer_english))
        .or_else(|| {
            value
                .get("text")
                .and_then(|text| text.as_str())
                .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
                .and_then(|inner| tail_read_range_answer_from_value(&inner, prefer_english))
        })
}

fn flat_tail_read_range_answer_from_value(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<String> {
    if !flat_value_is_tail_read_range(value) {
        return None;
    }
    let mut candidate = value.clone();
    let obj = candidate.as_object_mut()?;
    obj.insert(
        "action".to_string(),
        serde_json::Value::String("read_range".to_string()),
    );
    if !obj.contains_key("requested_n") {
        obj.insert(
            "requested_n".to_string(),
            serde_json::Value::Number(tail_read_requested_n(value)?.into()),
        );
    }
    crate::agent_engine::observed_output::tail_read_range_direct_answer_candidate(
        &candidate.to_string(),
        prefer_english,
    )
}

fn normalized_tail_read_range_lines_from_value(
    value: &serde_json::Value,
    prefer_english: bool,
) -> Option<Vec<String>> {
    if let Some(lines) = flat_normalized_tail_read_range_lines_from_value(value, prefer_english) {
        return Some(lines);
    }
    value
        .get("extra")
        .and_then(|extra| normalized_tail_read_range_lines_from_value(extra, prefer_english))
        .or_else(|| {
            value
                .get("text")
                .and_then(|text| text.as_str())
                .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
                .and_then(|inner| {
                    normalized_tail_read_range_lines_from_value(&inner, prefer_english)
                })
        })
}

fn flat_normalized_tail_read_range_lines_from_value(
    value: &serde_json::Value,
    _prefer_english: bool,
) -> Option<Vec<String>> {
    if !flat_value_is_tail_read_range(value) {
        return None;
    }
    let excerpt = value.get("excerpt").and_then(|value| value.as_str())?;
    let lines = crate::agent_engine::observed_output::normalize_read_range_excerpt(excerpt)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    (!lines.is_empty()).then_some(lines)
}

pub(crate) fn selected_tail_read_range_line_from_step_output(
    output: &str,
    prefer_english: bool,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    let lines = normalized_tail_read_range_lines_from_value(&value, prefer_english)?;
    select_tail_read_range_line(&lines)
}

fn tail_line_attention_score(line: &str) -> u8 {
    let upper = line.to_ascii_uppercase();
    if upper.contains(" ERROR ") || upper.contains(" ERROR:") || upper.contains("LEVEL=ERROR") {
        return 100;
    }
    if upper.contains(" WARN ") || upper.contains(" WARN:") || upper.contains("LEVEL=WARN") {
        return 90;
    }
    if line.contains("answer_verifier_observed_gap") {
        return 86;
    }
    if line.contains("verifier_result") {
        return 84;
    }
    if line.contains("loop_round_extend") || line.contains("synthesize_answer_failed") {
        return 78;
    }
    if line.contains("task_call:") {
        return 60;
    }
    10
}

fn line_contains_internal_redaction(line: &str) -> bool {
    line.contains("[INTERNAL_CONTEXT_REDACTED]")
}

fn select_tail_read_range_line(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .filter(|line| !line_contains_internal_redaction(line))
        .enumerate()
        .max_by_key(|(idx, line)| (tail_line_attention_score(line), *idx))
        .map(|(_, line)| line.trim().to_string())
        .or_else(|| {
            lines
                .iter()
                .enumerate()
                .max_by_key(|(idx, line)| (tail_line_attention_score(line), *idx))
                .map(|(_, line)| line.trim().to_string())
        })
        .filter(|line| !line.is_empty())
}

pub(super) fn latest_tail_read_range_selected_line_from_loop(
    loop_state: &LoopState,
    prefer_english: bool,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
                return None;
            }
            let output = step.output.as_deref()?.trim();
            selected_tail_read_range_line_from_step_output(output, prefer_english)
        })
}

pub(super) fn current_user_visible_delivery_text(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .delivery_messages
        .iter()
        .rev()
        .find_map(|message| {
            let text = message.trim();
            (!text.is_empty() && !crate::finalize::is_execution_summary_message(text))
                .then_some(text)
        })
        .or_else(|| {
            loop_state
                .last_user_visible_respond
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
        })
}

fn latest_tail_replacement_can_recover_stale_synthesis(
    loop_state: &LoopState,
    current_delivery: &str,
) -> bool {
    let current_delivery = current_delivery.trim();
    let Some(synthesis_idx) = loop_state.executed_step_results.iter().rposition(|step| {
        step.skill == "synthesize_answer"
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| output == current_delivery)
    }) else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .rposition(step_output_is_tail_read_range)
        .is_some_and(|tail_idx| tail_idx > synthesis_idx)
}

fn current_delivery_is_latest_registered_output(
    loop_state: &LoopState,
    current_delivery: &str,
) -> bool {
    loop_state
        .last_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|last_output| last_output == current_delivery.trim())
}

fn latest_tail_replacement_was_synthesized_after_tail(
    loop_state: &LoopState,
    replacement_answer: &str,
) -> bool {
    let Some(tail_idx) = loop_state
        .executed_step_results
        .iter()
        .rposition(step_output_is_tail_read_range)
    else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .skip(tail_idx + 1)
        .any(|(_, step)| {
            step.skill == "synthesize_answer"
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|output| output == replacement_answer.trim())
        })
}

fn latest_tail_read_range_should_preserve_current_delivery(
    route: Option<&crate::RouteResult>,
    loop_state: &LoopState,
    replacement_answer: &str,
) -> bool {
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    if current_delivery.trim() == replacement_answer.trim() {
        return false;
    }
    if latest_tail_replacement_can_recover_stale_synthesis(loop_state, current_delivery) {
        return false;
    }
    if route_requires_raw_tail_read_passthrough(route) {
        return false;
    }
    if route_prefers_deterministic_tail_line(route) {
        return false;
    }
    if latest_tail_replacement_was_synthesized_after_tail(loop_state, current_delivery) {
        return true;
    }
    if latest_tail_replacement_was_synthesized_after_tail(loop_state, replacement_answer) {
        return false;
    }
    if looks_like_structured_machine_output(current_delivery) {
        return false;
    }
    if current_delivery_is_latest_registered_output(loop_state, current_delivery) {
        return true;
    }
    route
        .map(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::ContentExcerptSummary
        })
        .unwrap_or(false)
}

fn semantic_kind_prefers_deterministic_tail_line(kind: crate::OutputSemanticKind) -> bool {
    matches!(
        kind,
        crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ExcerptKindJudgment
    )
}

fn route_prefers_deterministic_tail_line(route: Option<&crate::RouteResult>) -> bool {
    route
        .map(|route| {
            route.output_contract.response_shape == crate::OutputResponseShape::OneSentence
                && semantic_kind_prefers_deterministic_tail_line(
                    route.output_contract.semantic_kind,
                )
                && route.output_contract.requires_content_evidence
                && !route.output_contract.delivery_required
        })
        .unwrap_or(false)
}

pub(super) fn route_requires_raw_tail_read_passthrough(route: Option<&crate::RouteResult>) -> bool {
    route
        .map(|route| {
            route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
                && route.output_contract.response_shape == crate::OutputResponseShape::Strict
                && route.output_contract.requires_content_evidence
                && !route.output_contract.delivery_required
        })
        .unwrap_or(false)
}

pub(super) fn replace_delivery_with_latest_tail_read_range_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) = latest_tail_read_range_observed_answer(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
    ) else {
        return false;
    };
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    if latest_tail_read_range_should_preserve_current_delivery(route, loop_state, &answer) {
        info!(
            "delivery keep_current_summary_over_tail_read_range task_id={}",
            task.task_id
        );
        log_deterministic_delivery_record(
            &task.task_id,
            "keep_current_summary_over_tail_read_range",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return false;
    }
    if loop_state
        .delivery_messages
        .last()
        .map(|message| message.trim() == answer.trim())
        .unwrap_or(false)
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "replace_with_latest_tail_read_range",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

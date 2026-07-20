use std::collections::BTreeSet;
use std::path::Path;

use tracing::info;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::delivery_utils::trim_path_token;
use crate::ClaimedTask;

use super::{
    current_user_visible_delivery_text, latest_bounded_read_range_answer_from_loop,
    latest_publishable_synthesis_step_matches, latest_successful_observation_body,
    latest_successful_raw_observation_body, log_deterministic_delivery_record,
    looks_like_raw_command_snapshot, looks_like_structured_machine_output,
    message_is_non_answer_separator, planned_delivery_is_publishable_model_language_answer,
    raw_command_output_needs_structural_projection, route_allows_latest_tail_read_range_delivery,
    route_prefers_language_rendered_execution_failed_step, structured_json_values_from_output,
};

fn contractual_last_respond_delivery_value(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    let contract = route;
    let answer = loop_state
        .last_user_visible_respond
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    if raw_command_output_needs_structural_projection(route, loop_state) {
        return None;
    }
    let exact_single_line_observation =
        last_respond_matches_single_line_observation(loop_state, answer);
    if strict_raw_command_output_exact_observation_answer(route, loop_state, answer) {
        return Some(answer.to_string());
    }
    if crate::agent_engine::observed_output::route_requires_synthesized_delivery(route)
        && !exact_single_line_observation
    {
        return None;
    }
    let has_explicit_answer_contract = contract.delivery_required
        || !contract.semantic_kind_is_unclassified()
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::FileToken
                | crate::OutputResponseShape::OneSentence
                | crate::OutputResponseShape::Strict
        );
    if (!has_explicit_answer_contract && !exact_single_line_observation)
        || !loop_state.has_tool_or_skill_output
    {
        return None;
    }
    if crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
        || looks_like_structured_machine_output(answer)
        || looks_like_raw_command_snapshot(answer)
    {
        return None;
    }
    if last_respond_should_defer_to_publishable_evidence_summary(route, loop_state, answer) {
        return None;
    }
    match crate::output_contract_verifier::verify_output_contract(contract, answer, "") {
        crate::output_contract_verifier::OutputContractVerdict::Pass => Some(answer.to_string()),
        crate::output_contract_verifier::OutputContractVerdict::Reshape { reshaped, .. } => {
            Some(reshaped)
        }
        crate::output_contract_verifier::OutputContractVerdict::Reject { .. } => None,
    }
}

fn last_respond_should_defer_to_publishable_evidence_summary(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    answer: &str,
) -> bool {
    if answer.trim().is_empty() {
        return false;
    }
    let summary = valid_publishable_synthesis_output(loop_state)
        .or_else(|| latest_publishable_respond_step_output(loop_state));
    if route.semantic_kind_is_any(&[
        crate::OutputSemanticKind::RawCommandOutput,
        crate::OutputSemanticKind::CommandOutputSummary,
    ]) && !publishable_summary_has_multi_source_observation(loop_state)
    {
        return false;
    }
    match (
        crate::evidence_policy::final_answer_shape_for_output_contract(route),
        summary,
    ) {
        (Some(crate::evidence_policy::FinalAnswerShape::SummaryWithEvidence), Some(summary)) => {
            publishable_evidence_summary_should_own_delivery(summary)
        }
        (
            Some(crate::evidence_policy::FinalAnswerShape::RawOutputOrShortSummary),
            Some(summary),
        ) => publishable_evidence_summary_strictly_richer_than_answer(summary, answer),
        _ => false,
    }
}

fn publishable_evidence_summary_should_own_delivery(candidate: &str) -> bool {
    let candidate = candidate.trim();
    if !planned_delivery_is_publishable_model_language_answer(candidate) {
        return false;
    }
    let nonempty_lines = candidate
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let token_count = candidate.split_whitespace().count();
    let char_count = candidate.chars().count();
    nonempty_lines > 1 || token_count >= 8 || char_count >= 64
}

fn publishable_evidence_summary_strictly_richer_than_answer(summary: &str, answer: &str) -> bool {
    if !publishable_evidence_summary_should_own_delivery(summary) {
        return false;
    }
    let summary_chars = summary.trim().chars().count();
    let answer_chars = answer.trim().chars().count();
    summary_chars > answer_chars.saturating_add(16)
}

pub(super) fn publishable_summary_has_multi_source_observation(loop_state: &LoopState) -> bool {
    let mut sources = BTreeSet::new();
    for step in loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok())
    {
        if matches!(
            step.skill.as_str(),
            "respond" | "synthesize_answer" | "think"
        ) {
            continue;
        }
        if step
            .output
            .as_deref()
            .is_some_and(|output| !output.trim().is_empty())
        {
            sources.insert(step.skill.as_str());
        }
        if sources.len() >= 2 {
            return true;
        }
    }
    false
}

fn free_answer_route_allows_terminal_respond_delivery(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let contract = route.clone();
    !contract.delivery_required
        && !contract.requires_content_evidence
        && contract.response_shape == crate::OutputResponseShape::Free
        && route.semantic_kind_is_unclassified()
}

pub(crate) fn latest_publishable_respond_step_output(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "respond")
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .find(|output| planned_delivery_is_publishable_model_language_answer(output))
}

fn loop_has_non_control_observation(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().any(|step| {
        !matches!(
            step.skill.as_str(),
            "respond" | "synthesize_answer" | "think" | "answer_verifier"
        )
    })
}

fn terminal_respond_delivery_candidate(output: &str) -> bool {
    let output = output.trim();
    !output.is_empty()
        && !crate::finalize::looks_like_planner_artifact(output)
        && !crate::finalize::looks_like_internal_trace_artifact(output)
        && !crate::finalize::is_execution_summary_message(output)
}

fn terminal_respond_step_without_observed_execution(loop_state: &LoopState) -> Option<&str> {
    if loop_has_non_control_observation(loop_state) {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "respond")
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .find(|output| terminal_respond_delivery_candidate(output))
}

pub(super) fn last_respond_matches_single_line_observation(
    loop_state: &LoopState,
    answer: &str,
) -> bool {
    let Some(body) = latest_successful_observation_body(loop_state) else {
        return false;
    };
    let mut lines = body.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(line) = lines.next() else {
        return false;
    };
    if lines.next().is_some() || answer.trim() != line {
        return false;
    }
    !looks_like_structured_machine_output(line)
        && !looks_like_raw_command_snapshot(line)
        && !crate::finalize::looks_like_planner_artifact(line)
        && !crate::finalize::looks_like_internal_trace_artifact(line)
}

pub(super) fn strict_raw_command_output_exact_observation_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    answer: &str,
) -> bool {
    let contract = route.clone();
    if !route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        || contract.response_shape != crate::OutputResponseShape::Strict
        || !contract.requires_content_evidence
        || contract.delivery_required
        || raw_command_output_needs_structural_projection(route, loop_state)
    {
        return false;
    }
    let answer = answer.trim();
    if answer.is_empty()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || looks_like_structured_machine_output(answer)
    {
        return false;
    }
    latest_successful_raw_observation_body(loop_state)
        .map(str::trim)
        .is_some_and(|body| !body.is_empty() && body == answer)
        || latest_successful_observation_body(loop_state)
            .as_deref()
            .map(str::trim)
            .is_some_and(|body| !body.is_empty() && body == answer)
}

pub(super) fn candidate_matches_successful_external_observation(
    loop_state: &LoopState,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    !candidate.is_empty()
        && loop_state.executed_step_results.iter().any(|step| {
            if !step.is_ok()
                || matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
            {
                return false;
            }
            step.output.as_deref().is_some_and(|output| {
                crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                    output,
                )
                .trim()
                    == candidate
            })
        })
}

fn candidate_matches_structured_locator_observation(
    loop_state: &LoopState,
    candidate: &str,
) -> bool {
    let candidate = trim_path_token(candidate);
    if candidate.is_empty() || candidate.contains('\n') {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
            && step.output.as_deref().is_some_and(|output| {
                structured_json_values_from_output(output)
                    .iter()
                    .any(|value| value_contains_locator_candidate(value, &candidate))
            })
    })
}

fn value_contains_locator_candidate(value: &serde_json::Value, candidate: &str) -> bool {
    match value {
        serde_json::Value::Object(obj) => obj.iter().any(|(key, child)| {
            let key_is_locator = matches!(
                key.as_str(),
                "path"
                    | "resolved_path"
                    | "requested_path"
                    | "output_path"
                    | "title"
                    | "file_name"
                    | "filename"
                    | "name"
            );
            if key_is_locator && scalar_value_matches_locator_candidate(child, candidate) {
                return true;
            }
            value_contains_locator_candidate(child, candidate)
        }),
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| value_contains_locator_candidate(item, candidate)),
        _ => false,
    }
}

fn scalar_value_matches_locator_candidate(value: &serde_json::Value, candidate: &str) -> bool {
    let Some(raw) = value.as_str().map(trim_path_token) else {
        return false;
    };
    if raw.is_empty() {
        return false;
    }
    raw == candidate
        || Path::new(&raw)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == candidate)
}

pub(super) fn backfill_delivery_from_last_outputs(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    let prefer_language_rendered_failed_step =
        route_prefers_language_rendered_execution_failed_step(agent_run_context);
    if loop_state.delivery_messages.is_empty() && prefer_language_rendered_failed_step {
        if let Some(answer) = loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .filter(|answer| {
                planned_delivery_is_publishable_model_language_answer(answer)
                    && !candidate_matches_successful_external_observation(loop_state, answer)
            })
            .map(ToString::to_string)
        {
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "final_result_use_failed_step_last_respond",
                "backfilled",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
        return;
    }

    if loop_state.delivery_messages.is_empty() {
        if backfill_latest_tail_read_range_delivery(task, loop_state, agent_run_context) {
            return;
        }
    }

    if loop_state.delivery_messages.is_empty()
        && structured_dry_run_generated_output_present(loop_state)
    {
        log_deterministic_delivery_record(
            &task.task_id,
            "final_result_defer_structured_dry_run_projection",
            "deferred",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }

    if loop_state.delivery_messages.is_empty() {
        if backfill_synthesis_for_content_evidence_delivery(task, loop_state, agent_run_context) {
            return;
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(answer) = contractual_last_respond_delivery_value(loop_state, agent_run_context)
        {
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "final_result_use_contractual_last_respond",
                "backfilled",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty()
        && free_answer_route_allows_terminal_respond_delivery(agent_run_context)
    {
        if let Some(answer) = latest_publishable_respond_step_output(loop_state).map(str::to_string)
        {
            append_delivery_message(
                &task.task_id,
                &mut loop_state.delivery_messages,
                answer.clone(),
            );
            loop_state.last_user_visible_respond = Some(answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "final_result_use_free_answer_respond_step",
                "backfilled",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty()
        && free_answer_route_allows_terminal_respond_delivery(agent_run_context)
    {
        if let Some(answer) =
            terminal_respond_step_without_observed_execution(loop_state).map(str::to_string)
        {
            append_delivery_message(
                &task.task_id,
                &mut loop_state.delivery_messages,
                answer.clone(),
            );
            loop_state.last_user_visible_respond = Some(answer);
            log_deterministic_delivery_record(
                &task.task_id,
                "final_result_use_terminal_respond_without_observed_execution",
                "backfilled",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(last_synthesis_output) =
            valid_publishable_synthesis_output(loop_state).map(str::to_string)
        {
            append_delivery_message(
                &task.task_id,
                &mut loop_state.delivery_messages,
                last_synthesis_output.clone(),
            );
            loop_state.last_user_visible_respond = Some(last_synthesis_output);
            log_deterministic_delivery_record(
                &task.task_id,
                "final_result_use_synthesis_output",
                "backfilled",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if agent_run_context
            .and_then(|ctx| ctx.output_contract())
            .is_some_and(|route| raw_command_output_needs_structural_projection(route, loop_state))
            && loop_state
                .last_user_visible_respond
                .as_deref()
                .is_some_and(|answer| {
                    crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
                        answer, loop_state,
                    )
                })
        {
            return;
        }
        if let Some(ref last_respond) = loop_state.last_user_visible_respond {
            if !last_respond.trim().is_empty() {
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    last_respond.clone(),
                );
                log_deterministic_delivery_record(
                    &task.task_id,
                    "final_result_use_last_respond",
                    "backfilled",
                    agent_run_context,
                    loop_state.executed_step_results.len(),
                );
            }
        }
    }
}

fn structured_dry_run_generated_output_present(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().rev().any(|step| {
        if !step.is_ok()
            || matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        {
            return false;
        }
        let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            return false;
        };
        let extra = value.get("extra").unwrap_or(&value);
        extra.get("dry_run").and_then(serde_json::Value::as_bool) == Some(true)
            && (json_string_field_present(extra.get("output_path"))
                || planned_output_path_present(extra.get("planned_outputs")))
    })
}

fn json_string_field_present(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

fn planned_output_path_present(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|item| json_string_field_present(item.get("path")))
}

fn backfill_synthesis_for_content_evidence_delivery(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if !route_expects_synthesis_over_raw_observation(route) {
        return false;
    }
    let Some(answer) = valid_publishable_synthesis_output(loop_state)
        .map(str::trim)
        .filter(|text| {
            !text.is_empty()
                && !crate::finalize::looks_like_planner_artifact(text)
                && !crate::finalize::looks_like_internal_trace_artifact(text)
                && crate::finalize::parse_delivery_token(text).is_none()
        })
        .map(str::to_string)
    else {
        return false;
    };
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    log_deterministic_delivery_record(
        &task.task_id,
        "final_result_use_content_evidence_synthesis",
        "backfilled",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn backfill_latest_tail_read_range_delivery(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if !route_allows_latest_tail_read_range_delivery(route) {
        return false;
    }
    let Some(answer) = latest_bounded_read_range_answer_from_loop(loop_state, false) else {
        return false;
    };
    if answer.trim().is_empty() {
        return false;
    }
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    log_deterministic_delivery_record(
        &task.task_id,
        "final_result_use_latest_tail_read_range",
        "backfilled",
        agent_run_context,
        1,
    );
    true
}

fn is_bare_template_placeholder(text: &str) -> bool {
    let trimmed = text.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return false;
    }
    let inner = trimmed[2..trimmed.len() - 2].trim();
    !inner.is_empty() && !inner.contains("{{") && !inner.contains("}}")
}

pub(super) fn replace_placeholder_delivery_with_synthesis(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
) {
    let Some(synthesis) = valid_publishable_synthesis_output(loop_state).map(str::to_string) else {
        return;
    };
    let Some(last_delivery) = loop_state.delivery_messages.last().map(String::as_str) else {
        return;
    };
    if !is_bare_template_placeholder(last_delivery) {
        return;
    }
    info!(
        "final_result_replace_placeholder_delivery_with_synthesis task_id={} placeholder={}",
        task.task_id,
        crate::truncate_for_log(last_delivery)
    );
    loop_state.delivery_messages.pop();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        synthesis.clone(),
    );
    loop_state.last_user_visible_respond = Some(synthesis);
}

pub(super) fn replace_raw_read_delivery_with_synthesis(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let contract = route.clone();
    if !contract.requires_content_evidence
        || contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || (route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
            && contract.response_shape == crate::OutputResponseShape::Strict)
    {
        return false;
    }
    let Some(synthesis) = valid_publishable_synthesis_output(loop_state).map(str::to_string) else {
        return false;
    };
    if crate::finalize::looks_like_planner_artifact(&synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(&synthesis)
        || crate::finalize::parse_delivery_token(&synthesis).is_some()
    {
        return false;
    }
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    if current_delivery == synthesis
        || !delivery_is_raw_read_observation(current_delivery, loop_state)
    {
        return false;
    }

    info!(
        "final_result_replace_raw_read_delivery_with_synthesis task_id={} raw={}",
        task.task_id,
        crate::truncate_for_log(current_delivery)
    );
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        synthesis.clone(),
    );
    loop_state.last_user_visible_respond = Some(synthesis);
    true
}

pub(super) fn replace_raw_observation_delivery_with_synthesis(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let contract = route.clone();
    if !contract.requires_content_evidence || contract.delivery_required {
        return false;
    }
    if !route_expects_synthesis_over_raw_observation(route) {
        return false;
    }
    let Some(synthesis) = valid_publishable_synthesis_output(loop_state).map(str::to_string) else {
        return false;
    };
    if crate::finalize::looks_like_planner_artifact(&synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(&synthesis)
    {
        return false;
    }
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    let delivery_matches_external_observation =
        crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
            current_delivery,
            loop_state,
        ) || candidate_matches_successful_external_observation(loop_state, current_delivery)
            || candidate_matches_structured_locator_observation(loop_state, current_delivery);
    if current_delivery == synthesis || !delivery_matches_external_observation {
        return false;
    }

    info!(
        "final_result_replace_raw_observation_delivery_with_synthesis task_id={} raw={}",
        task.task_id,
        crate::truncate_for_log(current_delivery)
    );
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        synthesis.clone(),
    );
    loop_state.last_user_visible_respond = Some(synthesis);
    true
}

pub(super) fn valid_publishable_synthesis_output(loop_state: &LoopState) -> Option<&str> {
    if let Some(synthesis) = latest_publishable_synthesis_step_output(loop_state) {
        return Some(synthesis);
    }
    if !latest_publishable_synthesis_step_matches(loop_state) {
        return None;
    }
    loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

pub(super) fn latest_publishable_synthesis_step_output(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "synthesize_answer")
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .find(|output| {
            !output.is_empty()
                && (planned_delivery_is_publishable_model_language_answer(output)
                    || lifecycle_result_synthesis_payload_is_publishable(output)
                    || strict_json_projection_synthesis_payload_is_publishable(loop_state, output))
                && !crate::finalize::is_execution_summary_message(output)
        })
}

fn lifecycle_result_synthesis_payload_is_publishable(output: &str) -> bool {
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    payload
        .pointer("/final_answer_shape")
        .and_then(serde_json::Value::as_str)
        == Some("lifecycle_result")
        && payload
            .pointer("/status")
            .and_then(serde_json::Value::as_str)
            == Some("ok")
        && payload
            .pointer("/steps")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|steps| !steps.is_empty())
        && payload
            .pointer("/final_state/cleanup_observed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

fn strict_json_projection_synthesis_payload_is_publishable(
    loop_state: &LoopState,
    output: &str,
) -> bool {
    let output = output.trim();
    if output.is_empty()
        || crate::finalize::looks_like_planner_artifact(output)
        || crate::finalize::looks_like_internal_trace_artifact(output)
    {
        return false;
    }
    if loop_state
        .output_vars
        .get("agent_loop.strict_json_projection_publishable")
        .map(String::as_str)
        != Some("true")
    {
        return false;
    }
    if loop_state
        .output_vars
        .get("agent_loop.strict_json_projection_output")
        .map(|value| value.trim())
        != Some(output)
    {
        return false;
    }
    let Ok(serde_json::Value::Object(object)) = serde_json::from_str::<serde_json::Value>(output)
    else {
        return false;
    };
    !object.is_empty()
        && object.len() <= 16
        && object
            .iter()
            .all(|(key, value)| valid_projection_json_key(key) && json_value_has_payload(value))
}

fn valid_projection_json_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn json_value_has_payload(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(text) => !text.trim().is_empty(),
        serde_json::Value::Array(items) => {
            !items.is_empty() && items.iter().all(json_value_has_payload)
        }
        serde_json::Value::Object(object) => {
            !object.is_empty() && object.values().all(json_value_has_payload)
        }
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
}

pub(crate) fn latest_contractual_synthesis_output(loop_state: &LoopState) -> Option<&str> {
    if let Some(synthesis) = valid_publishable_synthesis_output(loop_state) {
        return Some(synthesis);
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.is_ok() && step.skill == "synthesize_answer")
        .and_then(|step| step.output.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

pub(super) fn current_delivery_is_latest_publishable_synthesis(
    loop_state: &LoopState,
    current_delivery: &str,
) -> bool {
    let Some(synthesis) = valid_publishable_synthesis_output(loop_state) else {
        return false;
    };
    let current_delivery = current_delivery.trim();
    !current_delivery.is_empty()
        && current_delivery == synthesis
        && !crate::finalize::looks_like_planner_artifact(synthesis)
        && !crate::finalize::looks_like_internal_trace_artifact(synthesis)
        && crate::finalize::parse_delivery_token(synthesis).is_none()
}

pub(crate) fn route_expects_synthesis_over_raw_observation(
    route: &crate::IntentOutputContract,
) -> bool {
    let contract = route.clone();
    if contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }
    if crate::agent_engine::observed_output::route_requires_synthesized_delivery(route) {
        return true;
    }
    if contract.requires_content_evidence
        && route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && contract.response_shape == crate::OutputResponseShape::Free
        && crate::evidence_policy::final_answer_shape_for_output_contract(&contract)
            .is_some_and(|shape| shape.allows_model_language())
    {
        return true;
    }
    if contract.requires_content_evidence
        && route.semantic_kind_is_any(&[
            crate::OutputSemanticKind::ContentExcerptSummary,
            crate::OutputSemanticKind::ContentExcerptWithSummary,
        ])
        && !matches!(
            contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return true;
    }
    let constrained_sentence_delivery = contract.response_shape
        == crate::OutputResponseShape::OneSentence
        || contract.exact_sentence_count.is_some();
    if !contract.requires_content_evidence || !constrained_sentence_delivery {
        return false;
    }
    route.semantic_kind_is_unclassified()
        || route.semantic_kind_is_any(&[
            crate::OutputSemanticKind::RawCommandOutput,
            crate::OutputSemanticKind::CommandOutputSummary,
            crate::OutputSemanticKind::ContentExcerptSummary,
            crate::OutputSemanticKind::ContentExcerptWithSummary,
        ])
}

pub(super) fn delivery_is_raw_read_observation(delivery: &str, loop_state: &LoopState) -> bool {
    let delivery = delivery.trim();
    if delivery.is_empty()
        || crate::finalize::is_execution_summary_message(delivery)
        || message_is_non_answer_separator(delivery)
    {
        return false;
    }
    raw_read_range_output(delivery)
        || read_range_excerpt_like(delivery)
        || (crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
            delivery, loop_state,
        ) && loop_state
            .executed_step_results
            .iter()
            .rev()
            .any(step_output_is_read_range))
}

pub(super) fn step_output_is_read_range(step: &crate::executor::StepExecutionResult) -> bool {
    if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
        return false;
    }
    step.output.as_deref().map(str::trim).is_some_and(|output| {
        structured_json_values_from_output(output)
            .iter()
            .any(read_range_output_value)
    })
}

fn raw_read_range_output(output: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(output.trim())
        .ok()
        .is_some_and(|value| read_range_output_value(&value))
}

fn read_range_output_value(value: &serde_json::Value) -> bool {
    matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("read_range" | "read_text_range")
    ) && value
        .get("excerpt")
        .and_then(|value| value.as_str())
        .is_some_and(|excerpt| !excerpt.trim().is_empty())
}

fn read_range_excerpt_like(output: &str) -> bool {
    let mut numbered_lines = 0usize;
    let mut total_lines = 0usize;
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        total_lines += 1;
        let Some((prefix, rest)) = line.split_once('|') else {
            continue;
        };
        if !rest.trim().is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
            numbered_lines += 1;
        }
    }
    total_lines >= 3 && numbered_lines >= 3
}

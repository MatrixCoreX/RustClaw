use tracing::info;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::ClaimedTask;

use super::{
    current_user_visible_delivery_text, latest_publishable_synthesis_step_matches,
    latest_successful_observation_body, latest_successful_raw_observation_body,
    latest_tail_read_range_answer_from_loop, log_deterministic_delivery_record,
    looks_like_raw_command_snapshot, looks_like_structured_machine_output,
    message_is_non_answer_separator, planned_delivery_is_publishable_model_language_answer,
    raw_command_output_needs_structural_projection, route_allows_latest_tail_read_range_delivery,
    route_prefers_language_rendered_execution_failed_step, structured_json_values_from_output,
};

fn contractual_last_respond_delivery_value(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    let contract = &route.output_contract;
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
        || !matches!(contract.semantic_kind, crate::OutputSemanticKind::None)
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
    match crate::output_contract_verifier::verify_output_contract(
        contract,
        answer,
        &route.resolved_intent,
    ) {
        crate::output_contract_verifier::OutputContractVerdict::Pass => Some(answer.to_string()),
        crate::output_contract_verifier::OutputContractVerdict::Reshape { reshaped, .. } => {
            Some(reshaped)
        }
        crate::output_contract_verifier::OutputContractVerdict::Reject { .. } => None,
    }
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
    route: &crate::RouteResult,
    loop_state: &LoopState,
    answer: &str,
) -> bool {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::RawCommandOutput
        || route.output_contract.response_shape != crate::OutputResponseShape::Strict
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
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
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
                && step.output.as_deref().is_some_and(|output| {
                    crate::agent_engine::observed_output::normalized_success_body_for_direct_answer(
                        output,
                    )
                    .trim()
                        == candidate
                })
        })
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

    if loop_state.delivery_messages.is_empty() {
        if let Some(ref last_synthesis_output) = loop_state.last_publishable_synthesis_output {
            if !last_synthesis_output.trim().is_empty() {
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    last_synthesis_output.clone(),
                );
                log_deterministic_delivery_record(
                    &task.task_id,
                    "final_result_use_synthesis_output",
                    "backfilled",
                    agent_run_context,
                    loop_state.executed_step_results.len(),
                );
            }
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .is_some_and(|route| raw_command_output_needs_structural_projection(route, loop_state))
            && loop_state
                .last_user_visible_respond
                .as_deref()
                .is_some_and(|answer| {
                    crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
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

fn backfill_latest_tail_read_range_delivery(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_allows_latest_tail_read_range_delivery(route) {
        return false;
    }
    let Some(answer) = latest_tail_read_range_answer_from_loop(loop_state, false) else {
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
    let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
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
        synthesis.to_string(),
    );
    loop_state.last_user_visible_respond = Some(synthesis.to_string());
}

pub(super) fn replace_raw_read_delivery_with_synthesis(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || (route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
            && route.output_contract.response_shape == crate::OutputResponseShape::Strict)
        || !latest_publishable_synthesis_step_matches(loop_state)
    {
        return false;
    }
    let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return false;
    };
    if crate::finalize::looks_like_planner_artifact(synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(synthesis)
        || crate::finalize::parse_delivery_token(synthesis).is_some()
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
        synthesis.to_string(),
    );
    loop_state.last_user_visible_respond = Some(synthesis.to_string());
    true
}

pub(super) fn replace_raw_observation_delivery_with_synthesis(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !latest_publishable_synthesis_step_matches(loop_state)
    {
        return false;
    }
    if !route_expects_synthesis_over_raw_observation(route) {
        return false;
    }
    let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return false;
    };
    if crate::finalize::looks_like_planner_artifact(synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(synthesis)
    {
        return false;
    }
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    let delivery_matches_external_observation =
        crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
            current_delivery,
            loop_state,
        ) || candidate_matches_successful_external_observation(loop_state, current_delivery);
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
        synthesis.to_string(),
    );
    loop_state.last_user_visible_respond = Some(synthesis.to_string());
    true
}

pub(super) fn valid_publishable_synthesis_output(loop_state: &LoopState) -> Option<&str> {
    if !latest_publishable_synthesis_step_matches(loop_state) {
        return None;
    }
    loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

pub(super) fn latest_contractual_synthesis_output(loop_state: &LoopState) -> Option<&str> {
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

pub(super) fn route_expects_synthesis_over_raw_observation(route: &crate::RouteResult) -> bool {
    if route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }
    if crate::agent_engine::observed_output::route_requires_synthesized_delivery(route) {
        return true;
    }
    if route.output_contract.requires_content_evidence
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route.output_contract.response_shape == crate::OutputResponseShape::Free
        && crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
            .is_some_and(|shape| shape.allows_model_language())
    {
        return true;
    }
    let constrained_sentence_delivery = route.output_contract.response_shape
        == crate::OutputResponseShape::OneSentence
        || route.output_contract.exact_sentence_count.is_some();
    if !route.output_contract.requires_content_evidence || !constrained_sentence_delivery {
        return false;
    }
    matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
            | crate::OutputSemanticKind::RawCommandOutput
            | crate::OutputSemanticKind::CommandOutputSummary
            | crate::OutputSemanticKind::GitRepositoryState
            | crate::OutputSemanticKind::DirectoryPurposeSummary
            | crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
            | crate::OutputSemanticKind::WorkspaceProjectSummary
    )
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
        || (crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
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

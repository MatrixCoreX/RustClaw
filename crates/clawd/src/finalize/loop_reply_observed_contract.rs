use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    current_delivery_is_latest_publishable_synthesis, current_user_visible_delivery_text,
    delivery_is_raw_read_observation, delivery_is_single_line_text,
    delivery_message_is_json_container, deterministic_missing_observed_target_answer,
    deterministic_scalar_markdown_heading_answer_from_loop, direct_raw_command_output_projection,
    latest_contractual_synthesis_output, latest_path_batch_facts_has_implicit_metadata_fields,
    latest_plan_requested_synthesis, latest_publishable_synthesis_step_matches,
    log_deterministic_delivery_record, looks_like_raw_command_snapshot,
    looks_like_structured_machine_output, matrix_candidate_satisfies_final_shape,
    planned_delivery_is_publishable_model_language_answer,
    raw_command_output_needs_structural_projection, route_explicitly_requests_command_result,
    route_prefers_observed_answer, route_requires_matrix_deterministic_final_answer,
    service_status_system_basic_info_answer,
};

pub(super) fn direct_scalar_observed_answer(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route_allows_direct_scalar_observed_answer(route) {
        return None;
    }
    if let Some(answer) = state.and_then(|state| {
        let user_text = route.resolved_intent.trim();
        deterministic_missing_observed_target_answer(
            state,
            user_text,
            loop_state,
            agent_run_context,
        )
    }) {
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    if let Some(answer) =
        deterministic_scalar_markdown_heading_answer_from_loop(loop_state, agent_run_context)
    {
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    let answer =
        if crate::agent_engine::observed_output::scalar_route_prefers_structured_observed_answer(
            route, loop_state,
        ) {
            state
            .and_then(|state| {
                crate::agent_engine::observed_output::extract_direct_answer_from_generic_output_i18n(
                    loop_state,
                    state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                crate::agent_engine::observed_output::extract_direct_answer_from_generic_output(
                    loop_state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                state.and_then(|state| {
                    crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output_i18n(
                        loop_state,
                        state,
                        agent_run_context,
                    )
                })
            })
            .or_else(|| {
                crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output(
                    loop_state,
                    agent_run_context,
                )
            })?
        } else {
            state
            .and_then(|state| {
                crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output_i18n(
                    loop_state,
                    state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output(
                    loop_state,
                    agent_run_context,
                )
            })?
        };
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            ..Default::default()
        },
    ))
}

fn latest_scalar_observed_answer_from_loop_contract(
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let contract = loop_state.output_contract.as_ref()?;
    if contract.response_shape != crate::OutputResponseShape::Scalar {
        return None;
    }
    let body = latest_successful_observation_body(loop_state)?;
    let mut lines = body.lines().map(str::trim).filter(|line| !line.is_empty());
    let answer = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    if answer.is_empty()
        || crate::finalize::parse_delivery_token(answer).is_some()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || looks_like_structured_machine_output(answer)
        || crate::finalize::is_execution_summary_message(answer)
    {
        return None;
    }
    Some((
        answer.to_string(),
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: loop_state.executed_step_results.len(),
            ..Default::default()
        },
    ))
}

pub(super) fn latest_successful_observation_body(loop_state: &LoopState) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rfind(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        })
        .filter(|step| step.is_ok())
        .and_then(|step| step.output.as_deref())
        .map(crate::agent_engine::observed_output::normalized_success_body_for_direct_answer)
}

pub(super) fn latest_successful_raw_observation_body(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .executed_step_results
        .iter()
        .rfind(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        })
        .filter(|step| step.is_ok())
        .and_then(|step| step.output.as_deref())
}

fn latest_path_observed_answer_from_loop_contract(
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let contract = loop_state.output_contract.as_ref()?;
    if !matches!(
        contract.semantic_kind,
        crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::DirectoryEntryGroups
    ) {
        return None;
    }
    let body = latest_successful_observation_body(loop_state)?;
    let value: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    let results = value.get("results").and_then(serde_json::Value::as_array)?;
    if results.len() != 1 {
        return None;
    }
    let answer = results
        .first()
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
    {
        return None;
    }
    Some((
        answer.to_string(),
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: loop_state.executed_step_results.len(),
            ..Default::default()
        },
    ))
}

#[derive(Clone, Copy)]
enum LoopContractObservedAnswerKind {
    Scalar,
    PathList,
}

fn loop_contract_observed_answer_satisfies_required_evidence(
    loop_state: &LoopState,
    answer_kind: LoopContractObservedAnswerKind,
) -> bool {
    let Some(output_contract) = loop_state.output_contract.as_ref() else {
        return false;
    };
    let required_fields =
        crate::task_contract::required_evidence_fields_for_output_contract(output_contract);
    if required_fields.is_empty() {
        return true;
    }
    required_fields.iter().all(|field| match field.as_str() {
        "field_value" | "count" | "command_output" => {
            matches!(answer_kind, LoopContractObservedAnswerKind::Scalar)
        }
        "candidates" | "path" => matches!(answer_kind, LoopContractObservedAnswerKind::PathList),
        _ => false,
    })
}

pub(super) fn replace_delivery_with_direct_scalar_observed_answer(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if !agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(route_allows_direct_scalar_observed_answer)
    {
        return false;
    }
    let Some((answer, summary)) =
        direct_scalar_observed_answer(Some(state), loop_state, agent_run_context)
    else {
        return false;
    };
    if current_user_visible_delivery_text(loop_state)
        .map(str::trim)
        .is_some_and(|delivery| delivery == answer.trim())
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    if current_delivery_is_latest_publishable_synthesis(loop_state, current_delivery)
        && planned_delivery_is_publishable_model_language_answer(current_delivery)
        && delivery_is_single_line_text(current_delivery)
    {
        return false;
    }
    if !scalar_contract_delivery_should_be_replaced_with_observed_scalar(current_delivery, &answer)
    {
        return false;
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
        "replace_structured_with_direct_scalar_observed",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn scalar_contract_delivery_should_be_replaced_with_observed_scalar(
    delivery: &str,
    answer: &str,
) -> bool {
    let delivery = delivery.trim();
    let answer = answer.trim();
    if delivery.is_empty() || answer.is_empty() || delivery == answer {
        return false;
    }
    delivery_message_is_json_container(delivery)
        || looks_like_structured_machine_output(delivery)
        || delivery
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            > 1
        || delivery.contains(answer)
}

pub(super) fn replace_delivery_with_direct_structured_observed_answer(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ServiceStatus | crate::OutputSemanticKind::RawCommandOutput
    ) {
        return false;
    }
    let projected =
        if route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput {
            direct_raw_command_output_projection(state, route, loop_state).or_else(|| {
                direct_structured_observed_answer(Some(state), loop_state, agent_run_context)
            })
        } else {
            direct_structured_observed_answer(Some(state), loop_state, agent_run_context)
        };
    let Some((answer, summary)) = projected else {
        return false;
    };
    let answer = answer.trim();
    if answer.is_empty()
        || crate::finalize::parse_delivery_token(answer).is_some()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
    {
        return false;
    }
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer)
    {
        loop_state.last_user_visible_respond = Some(answer.to_string());
        *finalizer_summary = Some(summary);
        return true;
    }
    loop_state
        .delivery_messages
        .retain(|message| crate::finalize::is_execution_summary_message(message));
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.to_string(),
    );
    loop_state.last_user_visible_respond = Some(answer.to_string());
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "replace_with_direct_structured_observed",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn replace_delivery_with_loop_contract_observed_answer(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary, answer_kind)) =
        latest_scalar_observed_answer_from_loop_contract(loop_state)
            .map(|(answer, summary)| (answer, summary, LoopContractObservedAnswerKind::Scalar))
            .or_else(|| {
                latest_path_observed_answer_from_loop_contract(loop_state).map(
                    |(answer, summary)| (answer, summary, LoopContractObservedAnswerKind::PathList),
                )
            })
    else {
        return false;
    };
    if latest_publishable_synthesis_step_matches(loop_state)
        && current_user_visible_delivery_text(loop_state).is_some_and(|delivery| {
            let delivery = delivery.trim();
            loop_state
                .last_publishable_synthesis_output
                .as_deref()
                .map(str::trim)
                .is_some_and(|synthesis| {
                    delivery == synthesis
                        && !delivery_is_raw_read_observation(delivery, loop_state)
                        && !crate::finalize::looks_like_planner_artifact(delivery)
                        && !crate::finalize::looks_like_internal_trace_artifact(delivery)
                        && crate::finalize::parse_delivery_token(delivery).is_none()
                })
        })
    {
        return false;
    }
    if !loop_contract_observed_answer_satisfies_required_evidence(loop_state, answer_kind) {
        return false;
    }
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if route_requires_matrix_deterministic_final_answer(route)
            && !matrix_candidate_satisfies_final_shape(
                task,
                &route.resolved_intent,
                loop_state,
                agent_run_context,
                Some(summary.clone()),
                route,
                &answer,
            )
        {
            return false;
        }
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
        "replace_with_loop_contract_observed",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn latest_terminal_planned_respond(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|round| round.plan_result.as_ref())
        .filter_map(|plan| plan.steps.last())
        .find_map(|step| {
            if step.action_type != "respond" && step.skill != "respond" {
                return None;
            }
            step.args
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|content| !content.is_empty())
        })
}

fn observed_json_scalar_matches_candidate(value: &serde_json::Value, candidate: &str) -> bool {
    match value {
        serde_json::Value::String(text) => text.trim() == candidate,
        serde_json::Value::Number(number) => number.to_string() == candidate,
        serde_json::Value::Bool(value) => value.to_string() == candidate,
        serde_json::Value::Array(items) => items
            .iter()
            .any(|value| observed_json_scalar_matches_candidate(value, candidate)),
        serde_json::Value::Object(map) => map
            .values()
            .any(|value| observed_json_scalar_matches_candidate(value, candidate)),
        serde_json::Value::Null => false,
    }
}

fn planned_terminal_respond_is_grounded_in_observation(
    loop_state: &LoopState,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || crate::agent_engine::observed_output::answer_is_direct_observation_passthrough(
            candidate, loop_state,
        )
    {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
        })
        .filter_map(|step| step.output.as_deref())
        .any(|output| {
            let output = output.trim();
            if output == candidate && !looks_like_structured_machine_output(output) {
                return true;
            }
            serde_json::from_str::<serde_json::Value>(output)
                .ok()
                .is_some_and(|value| observed_json_scalar_matches_candidate(&value, candidate))
        })
}

fn contractual_grounded_terminal_planned_respond(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    let candidate = latest_terminal_planned_respond(loop_state)?.trim();
    if candidate.is_empty()
        || candidate.contains("{{")
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
        || looks_like_structured_machine_output(candidate)
        || looks_like_raw_command_snapshot(candidate)
        || !planned_terminal_respond_is_grounded_in_observation(loop_state, candidate)
    {
        return None;
    }
    let answer = match crate::output_contract_verifier::verify_output_contract(
        &route.output_contract,
        candidate,
        &route.resolved_intent,
    ) {
        crate::output_contract_verifier::OutputContractVerdict::Pass => candidate.to_string(),
        crate::output_contract_verifier::OutputContractVerdict::Reshape { reshaped, .. } => {
            reshaped.trim().to_string()
        }
        crate::output_contract_verifier::OutputContractVerdict::Reject { .. } => return None,
    };
    if answer.is_empty() || looks_like_structured_machine_output(&answer) {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: loop_state.executed_step_results.len().max(1),
            ..Default::default()
        },
    ))
}

pub(super) fn replace_structured_delivery_with_grounded_terminal_respond(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if !loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| delivery_message_is_json_container(message))
    {
        return false;
    }
    let Some((answer, summary)) =
        contractual_grounded_terminal_planned_respond(loop_state, agent_run_context)
    else {
        return false;
    };
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
        "replace_structured_with_grounded_terminal_respond",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn contractual_grounded_latest_synthesis(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let synthesis = latest_contractual_synthesis_output(loop_state)?;
    if synthesis.chars().count() > 800
        || crate::finalize::looks_like_planner_artifact(synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(synthesis)
        || crate::finalize::is_execution_summary_message(synthesis)
        || crate::finalize::parse_delivery_token(synthesis).is_some()
        || looks_like_structured_machine_output(synthesis)
        || looks_like_raw_command_snapshot(synthesis)
    {
        return None;
    }
    let has_successful_observation = loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    });
    if !has_successful_observation {
        return None;
    }
    let answer = match crate::output_contract_verifier::verify_output_contract(
        &route.output_contract,
        synthesis,
        &route.resolved_intent,
    ) {
        crate::output_contract_verifier::OutputContractVerdict::Pass => synthesis.to_string(),
        crate::output_contract_verifier::OutputContractVerdict::Reshape { reshaped, .. } => {
            let reshaped = reshaped.trim();
            if reshaped.is_empty() {
                return None;
            }
            reshaped.to_string()
        }
        crate::output_contract_verifier::OutputContractVerdict::Reject { .. } => return None,
    };
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: loop_state.executed_step_results.len().max(1),
            ..Default::default()
        },
    ))
}

pub(super) fn replace_structured_delivery_with_grounded_synthesis(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if !loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| delivery_message_is_json_container(message))
    {
        return false;
    }
    let Some((answer, summary)) =
        contractual_grounded_latest_synthesis(loop_state, agent_run_context)
    else {
        return false;
    };
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
        "replace_structured_with_grounded_synthesis",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn direct_structured_observed_answer(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    direct_structured_observed_answer_impl(state, loop_state, agent_run_context, false)
}

pub(super) fn direct_structured_observed_answer_allowing_implicit_metadata_path_facts(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    direct_structured_observed_answer_impl(state, loop_state, agent_run_context, true)
}

fn direct_structured_observed_answer_impl(
    state: Option<&AppState>,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    allow_implicit_metadata_path_facts: bool,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if raw_command_output_needs_structural_projection(route, loop_state) {
        return None;
    }
    if let Some(answer) =
        crate::agent_engine::observed_output::structured_scalar_equality_direct_answer(
            state,
            route,
            loop_state,
            agent_run_context,
        )
    {
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                parsed: true,
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 2,
                ..Default::default()
            },
        ));
    }
    if route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && latest_plan_requested_synthesis(loop_state)
        && route.output_contract.semantic_kind != crate::OutputSemanticKind::GitRepositoryState
        && route.output_contract.semantic_kind != crate::OutputSemanticKind::ServiceStatus
    {
        return None;
    }
    if let Some(answer) = service_status_system_basic_info_answer(route, loop_state) {
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                parsed: true,
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }
    if route.ask_mode.finalize_chat_wrapped()
        && route.output_contract.requires_content_evidence
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && latest_path_batch_facts_has_implicit_metadata_fields(loop_state)
        && !allow_implicit_metadata_path_facts
    {
        return None;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) {
        return None;
    }
    if crate::agent_engine::observed_output::recent_structured_scalar_observation_count(loop_state)
        > 1
    {
        return None;
    }
    let successful_observation_count = loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|output| !output.is_empty())
        })
        .count();
    if route.output_contract.requires_content_evidence
        && successful_observation_count > 1
        && !route_prefers_observed_answer(route)
    {
        return None;
    }
    let answer = state
        .and_then(|state| {
            crate::agent_engine::observed_output::extract_direct_answer_from_generic_output_i18n(
                loop_state,
                state,
                agent_run_context,
            )
        })
        .or_else(|| {
            crate::agent_engine::observed_output::extract_direct_answer_from_generic_output(
                loop_state,
                agent_run_context,
            )
        })?;
    if answer.trim().is_empty() {
        return None;
    }
    if crate::agent_engine::observed_output::route_requires_synthesized_delivery(route) {
        let latest_raw_observation = loop_state
            .executed_step_results
            .iter()
            .rfind(|step| {
                step.is_ok()
                    && !matches!(
                        step.skill.as_str(),
                        "respond" | "synthesize_answer" | "think"
                    )
            })
            .and_then(|step| step.output.as_deref())
            .map(str::trim)
            .unwrap_or_default();
        if successful_observation_count != 1 || latest_raw_observation == answer.trim() {
            return None;
        }
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

pub(super) fn direct_non_builtin_skill_raw_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|text| !text.is_empty())
    {
        return None;
    }
    let last_skill_name = loop_state
        .output_vars
        .get("last_skill_name")
        .map(String::as_str)?;
    if state.is_builtin_skill(last_skill_name) {
        return None;
    }
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    if route.is_some_and(crate::agent_engine::observed_output::route_requires_synthesized_delivery)
    {
        return None;
    }
    let answer = loop_state
        .executed_step_results
        .iter()
        .rfind(|step| step.is_ok() && step.skill == last_skill_name)
        .and_then(|step| step.output.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())?
        .to_string();
    if direct_structured_observed_answer(None, loop_state, agent_run_context)
        .is_some_and(|(structured_answer, _)| structured_answer.trim() != answer.trim())
    {
        return None;
    }
    if matches!(
        route.map(|route| route.output_contract.response_shape),
        Some(crate::OutputResponseShape::Scalar)
    ) && !matches!(
        route.map(|route| route.output_contract.semantic_kind),
        Some(crate::OutputSemanticKind::RawCommandOutput)
    ) {
        return None;
    }
    if matches!(
        route.map(|route| route.output_contract.response_shape),
        Some(crate::OutputResponseShape::OneSentence)
    ) && !matches!(
        route.map(|route| route.output_contract.semantic_kind),
        Some(crate::OutputSemanticKind::RawCommandOutput)
    ) {
        return None;
    }
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
        || (looks_like_structured_machine_output(&answer)
            && !matches!(
                route.map(|route| route.output_contract.semantic_kind),
                Some(crate::OutputSemanticKind::RawCommandOutput)
            ))
    {
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

pub(super) fn route_allows_direct_scalar_observed_answer(route: &crate::RouteResult) -> bool {
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount {
        return true;
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::Scalar {
        return true;
    }
    route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.exact_sentence_count == Some(1)
        && !route.output_contract.delivery_required
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
}

pub(super) async fn direct_publishable_observed_answer(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return None;
    };
    if route_requires_matrix_deterministic_final_answer(route) {
        return None;
    }
    if route.output_contract.requires_content_evidence
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let observed =
        crate::agent_engine::observed_output::extract_latest_generic_successful_output(loop_state)?;
    let answer = observed.body.trim().to_string();
    if answer.is_empty()
        || crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
        || looks_like_structured_machine_output(&answer)
    {
        return None;
    }
    if observed.skill == "run_cmd" && !route_explicitly_requests_command_result(route) {
        return None;
    }
    if looks_like_raw_command_snapshot(&answer)
        && !(observed.skill == "run_cmd" && route_explicitly_requests_command_result(route))
    {
        return None;
    }
    let raw_command_passthrough =
        observed.skill == "run_cmd" && route_explicitly_requests_command_result(route);
    // §3.4 finalize-tier: observed_generic_finalize 是 finalize 决策层。
    if !raw_command_passthrough
        && !crate::semantic_judge::is_publishable_raw(state, task, &answer).await
    {
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

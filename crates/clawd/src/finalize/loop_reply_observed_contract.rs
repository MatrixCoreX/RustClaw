use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    current_delivery_is_latest_publishable_synthesis, current_user_visible_delivery_text,
    delivery_is_raw_read_observation, delivery_is_single_line_text,
    delivery_message_is_json_container, deterministic_missing_observed_target_answer,
    deterministic_scalar_markdown_heading_answer_from_loop, direct_raw_command_output_projection,
    evidence_policy_candidate_satisfies_final_shape, latest_contractual_synthesis_output,
    latest_path_batch_facts_has_implicit_metadata_fields, latest_plan_requested_synthesis,
    latest_publishable_respond_step_output, latest_publishable_synthesis_step_matches,
    log_deterministic_delivery_record, looks_like_raw_command_snapshot,
    looks_like_structured_machine_output, planned_delivery_is_publishable_model_language_answer,
    publishable_summary_has_multi_source_observation,
    raw_command_output_needs_structural_projection, route_explicitly_requests_command_result,
    route_prefers_observed_answer, route_requires_evidence_policy_deterministic_final_answer,
    service_status_system_basic_info_answer, valid_publishable_synthesis_output,
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
    if let Some((answer, summary)) =
        latest_terminal_scalar_respond_answer_from_loop_contract(route, loop_state)
    {
        return Some((answer, summary));
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
                crate::agent_engine::observed_output::extract_answer_from_observed_output_i18n(
                    loop_state,
                    state,
                    agent_run_context,
                )
            })
            .or_else(|| {
                crate::agent_engine::observed_output::extract_answer_from_observed_output(
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
    if scalar_projection_should_defer_to_publishable_evidence_summary(route, loop_state, &answer) {
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

fn scalar_projection_should_defer_to_publishable_evidence_summary(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    answer: &str,
) -> bool {
    let contract = route.effective_output_contract();
    let shape = crate::evidence_policy::final_answer_shape_for_route(route);
    if contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }
    let Some(summary) = valid_publishable_synthesis_output(loop_state)
        .or_else(|| latest_publishable_respond_step_output(loop_state))
    else {
        return false;
    };
    if !route_allows_publishable_summary_over_observed_projection(route, loop_state) {
        return false;
    }
    match shape {
        Some(crate::evidence_policy::FinalAnswerShape::SummaryWithEvidence) => {
            publishable_evidence_summary_should_own_scalar_delivery(summary)
        }
        Some(crate::evidence_policy::FinalAnswerShape::RawOutputOrShortSummary) => {
            publishable_evidence_summary_strictly_richer_than_scalar(summary, answer)
        }
        _ => false,
    }
}

fn route_allows_publishable_summary_over_observed_projection(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    if route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::RawCommandOutput,
        crate::OutputSemanticKind::CommandOutputSummary,
    ]) {
        return publishable_summary_has_multi_source_observation(loop_state);
    }
    route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::ContentExcerptSummary,
        crate::OutputSemanticKind::ContentExcerptWithSummary,
        crate::OutputSemanticKind::ExcerptKindJudgment,
        crate::OutputSemanticKind::WorkspaceProjectSummary,
    ]) && !matches!(
        route.effective_output_contract().response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    )
}

fn publishable_evidence_summary_should_own_scalar_delivery(candidate: &str) -> bool {
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

fn publishable_evidence_summary_strictly_richer_than_scalar(summary: &str, answer: &str) -> bool {
    if !publishable_evidence_summary_should_own_scalar_delivery(summary) {
        return false;
    }
    let summary_chars = summary.trim().chars().count();
    let answer_chars = answer.trim().chars().count();
    summary_chars > answer_chars.saturating_add(16)
}

fn latest_terminal_scalar_respond_answer_from_loop_contract(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || route.output_contract.delivery_required
        || matches!(
            route.effective_output_contract_semantic_kind(),
            crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::CommandOutputSummary
                | crate::OutputSemanticKind::ExecutionFailedStep
        )
    {
        return None;
    }
    let answer = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "respond")
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .find(|candidate| scalar_terminal_respond_candidate_matches_contract(route, candidate))?
        .to_string();
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

fn scalar_terminal_respond_candidate_matches_contract(
    route: &crate::RouteResult,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            != 1
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
        || crate::finalize::is_non_answer_separator_message(candidate)
        || looks_like_structured_machine_output(candidate)
        || looks_like_raw_command_snapshot(candidate)
        || machine_field_placeholder_delivery_for_scalar_contract(candidate, Some(route))
    {
        return false;
    }
    if crate::finalize::route_matches_single_path_output_contract(route)
        && !candidate_looks_like_path_scalar(candidate)
    {
        return false;
    }
    if crate::finalize::route_matches_single_path_output_contract(route)
        && candidate_looks_like_path_scalar(candidate)
    {
        return true;
    }
    let contract = route.effective_output_contract();
    matches!(
        crate::output_contract_verifier::verify_output_contract(
            &contract,
            candidate,
            &route.resolved_intent,
        ),
        crate::output_contract_verifier::OutputContractVerdict::Pass
    )
}

fn candidate_looks_like_path_scalar(candidate: &str) -> bool {
    if candidate.contains('\0') || candidate.chars().any(char::is_control) {
        return false;
    }
    let path = std::path::Path::new(candidate);
    path.is_absolute()
        || candidate.starts_with("./")
        || candidate.starts_with("../")
        || candidate.contains('/')
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
        .map(crate::agent_engine::observed_output::normalized_success_body_for_observed_output)
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
        crate::evidence_policy::required_evidence_fields_for_output_contract(output_contract);
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

fn current_delivery_is_publishable_terminal_answer(loop_state: &LoopState) -> bool {
    let Some(current) = current_user_visible_delivery_text(loop_state)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return false;
    };
    if !planned_delivery_is_publishable_model_language_answer(current) {
        return false;
    }
    [
        valid_publishable_synthesis_output(loop_state),
        latest_publishable_respond_step_output(loop_state),
        latest_contractual_synthesis_output(loop_state),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .any(|terminal| terminal == current)
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
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        let contract = route.effective_output_contract();
        if contract.requires_content_evidence && route.output_contract_is_unclassified() {
            if let Some(synthesis) = valid_publishable_synthesis_output(loop_state)
                .map(str::trim)
                .filter(|text| {
                    planned_delivery_is_publishable_model_language_answer(text)
                        && delivery_is_single_line_text(text)
                        && !machine_field_placeholder_delivery_for_scalar_contract(
                            text,
                            Some(route),
                        )
                })
                .map(str::to_string)
            {
                loop_state.delivery_messages.clear();
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    synthesis.clone(),
                );
                loop_state.last_user_visible_respond = Some(synthesis);
                *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
                    stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                    disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                    parsed: true,
                    contract_ok: true,
                    completion_ok: Some(true),
                    grounded_ok: Some(true),
                    format_ok: Some(true),
                    needs_clarify: Some(false),
                    used_evidence_ids_count: loop_state.executed_step_results.len(),
                    ..Default::default()
                });
                return true;
            }
        }
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
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    if current_delivery_is_latest_publishable_synthesis(loop_state, current_delivery)
        && planned_delivery_is_publishable_model_language_answer(current_delivery)
        && delivery_is_single_line_text(current_delivery)
        && !machine_field_placeholder_delivery_for_scalar_contract(current_delivery, route)
        && !recent_scalar_observed_answer_extends_delivery(current_delivery, &answer, route)
    {
        return false;
    }
    if !scalar_contract_delivery_should_be_replaced_with_observed_scalar(
        current_delivery,
        &answer,
        route,
    ) {
        return false;
    }
    loop_state.delivery_messages.clear();
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
    route: Option<&crate::RouteResult>,
) -> bool {
    let delivery = delivery.trim();
    let answer = answer.trim();
    if delivery.is_empty() || answer.is_empty() || delivery == answer {
        return false;
    }
    machine_field_placeholder_delivery_for_scalar_contract(delivery, route)
        || delivery_message_is_json_container(delivery)
        || looks_like_structured_machine_output(delivery)
        || delivery
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            > 1
        || recent_scalar_observed_answer_extends_delivery(delivery, answer, route)
        || delivery.contains(answer)
}

fn recent_scalar_observed_answer_extends_delivery(
    delivery: &str,
    answer: &str,
    route: Option<&crate::RouteResult>,
) -> bool {
    route.is_some_and(|route| {
        let contract = route.effective_output_contract();
        route.output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
            && !contract.delivery_required
    }) && answer.contains(delivery)
        && answer
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            > delivery
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
}

fn machine_field_placeholder_delivery_for_scalar_contract(
    delivery: &str,
    route: Option<&crate::RouteResult>,
) -> bool {
    route.is_some_and(route_allows_direct_scalar_observed_answer)
        && matches!(
            delivery.trim(),
            "field_value"
                | "value"
                | "value_text"
                | "path"
                | "resolved_path"
                | "command_output"
                | "count"
                | "total"
        )
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
    if !route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
        && !crate::finalize::route_matches_service_status_output_contract(route)
    {
        return false;
    }
    let projected = if route.output_contract_marker_is(crate::OutputSemanticKind::RawCommandOutput)
    {
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
    if current_delivery_should_preserve_publishable_summary_over_projection(
        route, loop_state, answer,
    ) {
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
    loop_state.delivery_messages.clear();
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

fn current_delivery_should_preserve_publishable_summary_over_projection(
    route: &crate::RouteResult,
    loop_state: &LoopState,
    projected_answer: &str,
) -> bool {
    if !matches!(
        crate::evidence_policy::final_answer_shape_for_route(route),
        Some(
            crate::evidence_policy::FinalAnswerShape::SummaryWithEvidence
                | crate::evidence_policy::FinalAnswerShape::RawOutputOrShortSummary
        )
    ) {
        return false;
    }
    let Some(current) = current_user_visible_delivery_text(loop_state).map(str::trim) else {
        return false;
    };
    if !route_allows_publishable_summary_over_observed_projection(route, loop_state) {
        return false;
    }
    publishable_evidence_summary_strictly_richer_than_scalar(current, projected_answer)
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
    if agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            !route_prefers_observed_answer(route)
                && current_delivery_is_publishable_terminal_answer(loop_state)
        })
    {
        return false;
    }
    if !loop_contract_observed_answer_satisfies_required_evidence(loop_state, answer_kind) {
        return false;
    }
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) {
        if route_requires_evidence_policy_deterministic_final_answer(route)
            && !evidence_policy_candidate_satisfies_final_shape(
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
    loop_state.delivery_messages.clear();
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
        || crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
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
    loop_state.delivery_messages.clear();
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
    loop_state.delivery_messages.clear();
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
        && route.effective_output_contract().requires_content_evidence
        && latest_plan_requested_synthesis(loop_state)
        && !route.output_contract_marker_is(crate::OutputSemanticKind::GitRepositoryState)
        && !crate::finalize::route_matches_service_status_output_contract(route)
        && latest_successful_inventory_name_list_answer(loop_state).is_none()
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
        && route.effective_output_contract().requires_content_evidence
        && route.output_contract_marker_is(crate::OutputSemanticKind::ExistenceWithPath)
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
    if let Some(answer) = latest_successful_inventory_name_list_answer(loop_state) {
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
    let answer = state
        .and_then(|state| {
            crate::agent_engine::observed_output::extract_answer_from_observed_output_i18n(
                loop_state,
                state,
                agent_run_context,
            )
        })
        .or_else(|| {
            crate::agent_engine::observed_output::extract_answer_from_observed_output(
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

fn latest_successful_inventory_name_list_answer(loop_state: &LoopState) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(|step| {
            if !step.is_ok()
                || matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
            {
                return None;
            }
            let Some(output) = step
                .output
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            else {
                return None;
            };
            let output =
                crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
                    output,
                );
            serde_json::from_str::<serde_json::Value>(&output)
                .ok()
                .and_then(|value| inventory_name_list_answer_from_value(&value))
        })
}

fn inventory_name_list_answer_from_value(value: &serde_json::Value) -> Option<String> {
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        if let Some(answer) = inventory_name_list_answer_from_value(extra) {
            return Some(answer);
        }
    }
    if value.get("action").and_then(serde_json::Value::as_str) != Some("inventory_dir") {
        return None;
    }
    let names = if value
        .get("names_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        inventory_string_array(value.get("names"))?
    } else if value
        .get("dirs_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        inventory_kind_names(value, "dirs")
            .or_else(|| inventory_string_array(value.get("names")))?
    } else if value
        .get("files_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        inventory_kind_names(value, "files")
            .or_else(|| inventory_string_array(value.get("names")))?
    } else {
        return None;
    };
    (!names.is_empty()).then(|| names.join("\n"))
}

fn inventory_kind_names(value: &serde_json::Value, kind: &str) -> Option<Vec<String>> {
    inventory_string_array(value.pointer(format!("/names_by_kind/{kind}").as_str()))
}

fn inventory_string_array(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    let names = value?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    (!names.is_empty()).then_some(names)
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
        route.map(|route| route.effective_output_contract_semantic_kind()),
        Some(crate::OutputSemanticKind::RawCommandOutput)
    ) {
        return None;
    }
    if matches!(
        route.map(|route| route.output_contract.response_shape),
        Some(crate::OutputResponseShape::OneSentence)
    ) && !matches!(
        route.map(|route| route.effective_output_contract_semantic_kind()),
        Some(crate::OutputSemanticKind::RawCommandOutput)
    ) {
        return None;
    }
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
        || (looks_like_structured_machine_output(&answer)
            && !matches!(
                route.map(|route| route.effective_output_contract_semantic_kind()),
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
    let contract = route.effective_output_contract();
    if route.output_contract_marker_is(crate::OutputSemanticKind::ScalarCount) {
        return true;
    }
    if route.output_contract_marker_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
        && !contract.delivery_required
    {
        return true;
    }
    if contract.response_shape == crate::OutputResponseShape::Scalar {
        return true;
    }
    contract.response_shape == crate::OutputResponseShape::Strict
        && contract.exact_sentence_count == Some(1)
        && !contract.delivery_required
        && route.output_contract_is_unclassified()
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
    if route_requires_evidence_policy_deterministic_final_answer(route) {
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

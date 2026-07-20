use tracing::info;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::AppState;

use super::{
    current_synthesis_satisfies_evidence_policy_shape, delivery_is_raw_read_observation,
    delivery_is_single_line_text, delivery_matches_latest_publishable_synthesis,
    delivery_message_is_json_object, deterministic_matrix_observed_shape_answer,
    direct_raw_command_output_projection, direct_scalar_observed_answer,
    direct_structured_observed_answer, evidence_policy_candidate_satisfies_final_shape,
    latest_publishable_respond_step_output, latest_publishable_synthesis_matches_written_file_path,
    latest_publishable_synthesis_step_matches, latest_successful_observation_body,
    log_deterministic_delivery_record, looks_like_raw_command_snapshot,
    looks_like_structured_machine_output, output_contract_requests_exact_delivery,
    planned_delivery_is_publishable_model_language_answer,
    publishable_summary_has_multi_source_observation,
    raw_command_output_needs_structural_projection, route_allows_model_language_final_answer,
    route_expects_synthesis_over_raw_observation, route_requires_file_token,
    route_requires_observed_output_projection, scalar_answer_from_json,
    synthetic_task_for_evidence_policy_shape_check,
};

pub(super) fn prefer_observed_answer_for_exact_contract(
    state: &AppState,
    task_id: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return;
    };
    if !route_prefers_observed_answer(route) || route_requires_file_token(agent_run_context) {
        return;
    }
    if delivery_messages.is_empty() {
        return;
    }
    if delivery_messages
        .last()
        .is_some_and(|message| delivery_message_is_json_object(message))
    {
        info!(
            "delivery exact_contract_keep_planned_json task_id={}",
            task_id
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_keep_planned_json",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    let has_prior_step_error = loop_state
        .executed_step_results
        .iter()
        .any(|step| matches!(step.status, crate::executor::StepExecutionStatus::Error));
    let allow_prior_step_error_replacement =
        route_allows_prior_step_error_observed_replacement(route);
    if has_prior_step_error && !allow_prior_step_error_replacement {
        return;
    }
    if !route.requests_exact_name_list()
        && route_expects_synthesis_over_raw_observation(route)
        && delivery_matches_latest_publishable_synthesis(loop_state, delivery_messages)
    {
        info!(
            "delivery exact_contract_keep_publishable_synthesis task_id={}",
            task_id
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_keep_publishable_synthesis",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    let current_delivery_is_publishable_synthesis =
        delivery_messages.last().is_some_and(|message| {
            loop_state
                .last_publishable_synthesis_output
                .as_deref()
                .map(str::trim)
                .is_some_and(|synthesis| synthesis == message.trim())
        });
    if current_delivery_is_publishable_synthesis
        && latest_publishable_synthesis_step_matches(loop_state)
        && route.semantic_kind_is_unclassified()
        && !route.requests_exact_name_list()
        && delivery_messages.last().is_some_and(|message| {
            planned_delivery_is_publishable_model_language_answer(message)
                && delivery_is_single_line_text(message)
        })
    {
        info!(
            "delivery exact_contract_keep_semantic_none_synthesis task_id={} answer={}",
            task_id,
            crate::truncate_for_log(
                delivery_messages
                    .last()
                    .map(String::as_str)
                    .unwrap_or_default()
            )
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_keep_semantic_none_synthesis",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    if current_delivery_is_publishable_synthesis
        && latest_publishable_synthesis_matches_written_file_path(loop_state)
    {
        info!(
            "delivery exact_contract_keep_written_file_path_synthesis task_id={} answer={}",
            task_id,
            crate::truncate_for_log(
                delivery_messages
                    .last()
                    .map(String::as_str)
                    .unwrap_or_default()
            )
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_keep_written_file_path_synthesis",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    if let Some((answer, summary)) = direct_raw_command_output_projection(state, route, loop_state)
    {
        if delivery_messages.last().is_some_and(|delivery| {
            should_keep_publishable_summary_over_raw_command_projection(
                route, loop_state, delivery, &answer,
            )
        }) {
            info!(
                "delivery exact_contract_keep_publishable_summary_over_raw_projection task_id={} answer={}",
                task_id,
                crate::truncate_for_log(
                    delivery_messages
                        .last()
                        .map(String::as_str)
                        .unwrap_or_default()
                )
            );
            log_deterministic_delivery_record(
                task_id,
                "exact_contract_keep_publishable_summary_over_raw_projection",
                "preserved",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            return;
        }
        if delivery_messages
            .last()
            .is_some_and(|message| message.trim() == answer.trim())
        {
            loop_state.last_user_visible_respond = Some(answer);
            *finalizer_summary = Some(summary);
            return;
        }
        info!(
            "delivery exact_contract_raw_command_projection task_id={} previous={} observed={}",
            task_id,
            crate::truncate_for_log(
                delivery_messages
                    .last()
                    .map(String::as_str)
                    .unwrap_or_default()
            ),
            crate::truncate_for_log(&answer)
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_raw_command_projection",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return;
    }
    if raw_command_output_needs_structural_projection(route, loop_state)
        && delivery_messages.last().is_some_and(|message| {
            let message = message.trim();
            !message.is_empty()
                && !crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
                    message, loop_state,
                )
                && matches!(
                    crate::output_contract_verifier::verify_output_contract(
                        route,
                        message,
                        "",
                    ),
                    crate::output_contract_verifier::OutputContractVerdict::Pass
                        | crate::output_contract_verifier::OutputContractVerdict::Reshape { .. }
                )
        })
    {
        info!(
            "delivery exact_contract_keep_structural_projection_answer task_id={} answer={}",
            task_id,
            crate::truncate_for_log(
                delivery_messages
                    .last()
                    .map(String::as_str)
                    .unwrap_or_default()
            )
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_keep_structural_projection_answer",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    if let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let scalar_value_contract = route.response_shape == crate::OutputResponseShape::Scalar;
        if delivery_messages
            .last()
            .map(|message| message.trim() == synthesis)
            .unwrap_or(false)
            && !(has_prior_step_error && allow_prior_step_error_replacement)
            && !scalar_value_contract
            && !route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
            && planned_delivery_is_explicit_contractual_answer(route, synthesis)
        {
            info!(
                "delivery exact_contract_keep_synthesis task_id={} answer={}",
                task_id,
                crate::truncate_for_log(synthesis)
            );
            log_deterministic_delivery_record(
                task_id,
                "exact_contract_keep_synthesis",
                "preserved",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            return;
        }
    }
    let scalar_value_contract = route.response_shape == crate::OutputResponseShape::Scalar;
    if current_delivery_is_publishable_synthesis
        && latest_publishable_synthesis_step_matches(loop_state)
        && !(has_prior_step_error && allow_prior_step_error_replacement)
        && !scalar_value_contract
        && !route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && !route_requires_observed_output_projection(route)
        && current_synthesis_satisfies_evidence_policy_shape(
            task_id,
            loop_state,
            agent_run_context,
            finalizer_summary.clone(),
            route,
            delivery_messages,
        )
        && delivery_messages.last().is_some_and(|message| {
            !delivery_is_raw_read_observation(message, loop_state)
                && !crate::finalize::looks_like_planner_artifact(message)
                && !crate::finalize::looks_like_internal_trace_artifact(message)
                && crate::finalize::parse_delivery_token(message).is_none()
        })
    {
        info!(
            "delivery exact_contract_keep_latest_synthesis task_id={} answer={}",
            task_id,
            crate::truncate_for_log(
                delivery_messages
                    .last()
                    .map(String::as_str)
                    .unwrap_or_default()
            )
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_keep_latest_synthesis",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    if !current_delivery_is_publishable_synthesis
        && !scalar_value_contract
        && !route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && delivery_messages
            .last()
            .is_some_and(|message| planned_delivery_is_explicit_contractual_answer(route, message))
    {
        info!(
            "delivery exact_contract_keep_planned_contractual_answer task_id={} answer={}",
            task_id,
            crate::truncate_for_log(
                delivery_messages
                    .last()
                    .map(String::as_str)
                    .unwrap_or_default()
            )
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_keep_planned_contractual_answer",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    if route.semantic_kind_is(crate::OutputSemanticKind::GeneratedFilePathReport)
        && latest_publishable_synthesis_step_matches(loop_state)
    {
        if let Some(synthesis) = loop_state
            .last_publishable_synthesis_output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            let current = delivery_messages.last().map(|message| message.trim());
            if current != Some(synthesis) {
                let synthetic_task = synthetic_task_for_evidence_policy_shape_check(task_id);
                let summary = crate::task_journal::TaskJournalFinalizerSummary {
                    stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                    disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                    contract_ok: true,
                    completion_ok: Some(true),
                    grounded_ok: Some(true),
                    format_ok: Some(true),
                    needs_clarify: Some(false),
                    used_evidence_ids_count: loop_state.executed_step_results.len(),
                    ..Default::default()
                };
                if evidence_policy_candidate_satisfies_final_shape(
                    &synthetic_task,
                    "",
                    loop_state,
                    agent_run_context,
                    Some(summary.clone()),
                    route,
                    synthesis,
                ) {
                    info!(
                        "delivery exact_contract_use_generated_file_path_synthesis task_id={} previous={} synthesis={}",
                        task_id,
                        crate::truncate_for_log(
                            delivery_messages
                                .last()
                                .map(String::as_str)
                                .unwrap_or_default()
                        ),
                        crate::truncate_for_log(synthesis)
                    );
                    log_deterministic_delivery_record(
                        task_id,
                        "exact_contract_use_generated_file_path_synthesis",
                        "replaced",
                        agent_run_context,
                        loop_state.executed_step_results.len(),
                    );
                    delivery_messages.clear();
                    delivery_messages.push(synthesis.to_string());
                    loop_state.last_user_visible_respond = Some(synthesis.to_string());
                    *finalizer_summary = Some(summary);
                    return;
                }
            }
        }
    }
    let synthetic_task = synthetic_task_for_evidence_policy_shape_check(task_id);
    let Some((answer, summary)) = deterministic_matrix_observed_shape_answer(
        state,
        &synthetic_task,
        "",
        loop_state,
        agent_run_context,
    )
    .or_else(|| direct_scalar_observed_answer(Some(state), loop_state, agent_run_context))
    .or_else(|| direct_structured_observed_answer(Some(state), loop_state, agent_run_context))
    .or_else(|| exact_contract_fallback_observed_answer(route, loop_state)) else {
        return;
    };
    let answer = answer.trim();
    if answer.is_empty()
        || crate::finalize::parse_delivery_token(answer).is_some()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
    {
        return;
    }
    if delivery_messages
        .last()
        .map(|message| message.trim() == answer)
        .unwrap_or(false)
    {
        loop_state.last_user_visible_respond = Some(answer.to_string());
        *finalizer_summary = Some(summary);
        return;
    }
    let current_delivery_is_replaceable_status_synthesis = has_prior_step_error
        && allow_prior_step_error_replacement
        && current_delivery_is_publishable_synthesis;
    if !current_delivery_is_replaceable_status_synthesis
        && delivery_messages.last().is_some_and(|message| {
            should_keep_latest_publishable_terminal_delivery_over_observed_projection(
                route, loop_state, message, answer,
            )
        })
    {
        info!(
            "delivery exact_contract_keep_publishable_terminal_delivery task_id={} observed={}",
            task_id,
            crate::truncate_for_log(answer)
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_keep_publishable_terminal_delivery",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }
    if !current_delivery_is_replaceable_status_synthesis
        && delivery_messages.last().is_some_and(|message| {
            should_keep_planned_delivery_over_observed_answer(route, message, answer)
        })
    {
        info!(
            "delivery exact_contract_keep_planned_delivery task_id={} observed={}",
            task_id,
            crate::truncate_for_log(answer)
        );
        log_deterministic_delivery_record(
            task_id,
            "exact_contract_keep_planned_delivery",
            "preserved",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return;
    }

    info!(
        "delivery exact_contract_from_observed task_id={} previous={} observed={}",
        task_id,
        crate::truncate_for_log(
            delivery_messages
                .last()
                .map(String::as_str)
                .unwrap_or_default()
        ),
        crate::truncate_for_log(answer)
    );
    log_deterministic_delivery_record(
        task_id,
        "exact_contract_from_observed",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    delivery_messages.clear();
    delivery_messages.push(answer.to_string());
    loop_state.last_user_visible_respond = Some(answer.to_string());
    *finalizer_summary = Some(summary);
}

fn should_keep_latest_publishable_terminal_delivery_over_observed_projection(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    delivery: &str,
    observed: &str,
) -> bool {
    let delivery = delivery.trim();
    if delivery.is_empty()
        || crate::finalize::parse_delivery_token(delivery).is_some()
        || crate::finalize::looks_like_planner_artifact(delivery)
        || crate::finalize::looks_like_internal_trace_artifact(delivery)
    {
        return false;
    }
    if route.delivery_required
        || matches!(
            route.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        || route_requires_observed_output_projection(route)
    {
        return false;
    }
    if !planned_delivery_is_publishable_model_language_answer(delivery)
        || !delivery_is_structurally_richer_than_observed_projection(delivery, observed)
        || !delivery_matches_latest_publishable_terminal(loop_state, delivery)
    {
        return false;
    }
    !route.requires_content_evidence || loop_has_structured_tool_evidence(loop_state)
}

fn loop_has_structured_tool_evidence(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
            && step
                .output
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .is_some_and(|output| serde_json::from_str::<serde_json::Value>(output).is_ok())
    })
}

fn delivery_matches_latest_publishable_terminal(loop_state: &LoopState, delivery: &str) -> bool {
    let delivery = delivery.trim();
    if delivery.is_empty() {
        return false;
    }
    let synthesis_matches = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|synthesis| synthesis == delivery)
        && latest_publishable_synthesis_step_matches(loop_state);
    let respond_matches = latest_publishable_respond_step_output(loop_state)
        .map(str::trim)
        .is_some_and(|respond| respond == delivery);
    synthesis_matches || respond_matches
}

fn exact_contract_fallback_observed_answer(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if raw_command_output_needs_structural_projection(route, loop_state) {
        return None;
    }
    let body_string = latest_successful_observation_body(loop_state)?;
    let body = body_string.trim();
    if body.is_empty()
        || crate::finalize::looks_like_planner_artifact(body)
        || crate::finalize::looks_like_internal_trace_artifact(body)
        || looks_like_raw_command_snapshot(body)
    {
        return None;
    }
    let candidate = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| exact_contract_answer_from_json(route, &value))
        .or_else(|| raw_command_exact_observation_fallback_allowed(route).then(|| body.to_string()))
        .or_else(|| single_line_observation_answer(route, body))?;
    let candidate =
        match crate::output_contract_verifier::verify_output_contract(route, &candidate, "") {
            crate::output_contract_verifier::OutputContractVerdict::Pass => candidate,
            crate::output_contract_verifier::OutputContractVerdict::Reshape {
                reshaped, ..
            } => reshaped,
            crate::output_contract_verifier::OutputContractVerdict::Reject { .. } => {
                if exact_fallback_candidate_is_machine_grounded(route, &candidate) {
                    candidate
                } else {
                    return None;
                }
            }
        };
    Some((
        candidate,
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
    ))
}

fn raw_command_exact_observation_fallback_allowed(route: &crate::IntentOutputContract) -> bool {
    route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && !route_expects_synthesis_over_raw_observation(route)
}

fn exact_fallback_candidate_is_machine_grounded(
    route: &crate::IntentOutputContract,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || crate::finalize::is_execution_summary_message(candidate)
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || looks_like_structured_machine_output(candidate)
        || looks_like_raw_command_snapshot(candidate)
    {
        return false;
    }
    if matches!(route.response_shape, crate::OutputResponseShape::Scalar) {
        let mut lines = candidate
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        return lines.next().is_some() && lines.next().is_none();
    }
    if route_path_locator_allows_observed_listing(route) {
        return candidate.lines().any(|line| !line.trim().is_empty());
    }
    matches!(
        route.semantic_kind,
        crate::OutputSemanticKind::ExistenceWithPath | crate::OutputSemanticKind::FilePaths
    ) && candidate.lines().any(|line| !line.trim().is_empty())
}

fn exact_contract_answer_from_json(
    route: &crate::IntentOutputContract,
    value: &serde_json::Value,
) -> Option<String> {
    if matches!(route.response_shape, crate::OutputResponseShape::Scalar) {
        return scalar_answer_from_json(value);
    }
    if matches!(
        route.semantic_kind,
        crate::OutputSemanticKind::FilePaths | crate::OutputSemanticKind::ExistenceWithPath
    ) || matches!(route.locator_kind, crate::OutputLocatorKind::Path)
    {
        return path_answer_from_json(value);
    }
    None
}

fn path_answer_from_json(value: &serde_json::Value) -> Option<String> {
    for key in ["results", "paths", "names", "items"] {
        if let Some(items) = value.get(key).and_then(|child| child.as_array()) {
            let lines = items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            if !lines.is_empty() {
                return Some(lines.join("\n"));
            }
        }
    }
    for key in ["path", "resolved_path", "file_path", "output_path"] {
        if let Some(text) = value
            .get(key)
            .and_then(|child| child.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            return Some(text.to_string());
        }
    }
    None
}

fn single_line_observation_answer(
    route: &crate::IntentOutputContract,
    body: &str,
) -> Option<String> {
    let mut lines = body.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    if !matches!(route.response_shape, crate::OutputResponseShape::Scalar) {
        return None;
    }
    Some(first.to_string())
}

fn planned_delivery_is_explicit_contractual_answer(
    route: &crate::IntentOutputContract,
    delivery: &str,
) -> bool {
    if route.requests_exact_name_list() {
        return false;
    }
    let delivery = delivery.trim();
    if delivery.is_empty()
        || crate::finalize::is_execution_summary_message(delivery)
        || crate::finalize::parse_delivery_token(delivery).is_some()
        || crate::finalize::looks_like_planner_artifact(delivery)
        || crate::finalize::looks_like_internal_trace_artifact(delivery)
    {
        return false;
    }
    matches!(
        crate::output_contract_verifier::verify_output_contract(route, delivery, ""),
        crate::output_contract_verifier::OutputContractVerdict::Pass
    ) && list_contract_candidate_is_line_list(route, delivery)
}

fn list_contract_candidate_is_line_list(
    route: &crate::IntentOutputContract,
    delivery: &str,
) -> bool {
    if !matches!(route.semantic_kind, crate::OutputSemanticKind::FilePaths) {
        return true;
    }
    let lines = delivery
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() > 1 {
        return true;
    }
    lines
        .first()
        .is_some_and(|line| !line.chars().any(char::is_whitespace))
}

pub(super) fn route_prefers_observed_answer(route: &crate::IntentOutputContract) -> bool {
    if output_contract_requests_exact_delivery(route) {
        return true;
    }
    if route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput) {
        return true;
    }
    if route_path_locator_allows_observed_listing(route) {
        return true;
    }
    let required_evidence_fields =
        crate::evidence_policy::required_evidence_fields_for_output_contract(route);
    if required_evidence_fields
        .iter()
        .any(|field| field == "content_excerpt")
    {
        return false;
    }
    if required_evidence_fields.is_empty() {
        return false;
    }
    let delivery_shape = crate::evidence_policy::delivery_shape_for_output_contract(route);
    let operation = crate::evidence_policy::operation_for_output_contract(route);
    match delivery_shape {
        crate::evidence_policy::EvidenceDeliveryShape::Raw
        | crate::evidence_policy::EvidenceDeliveryShape::List
        | crate::evidence_policy::EvidenceDeliveryShape::File => true,
        crate::evidence_policy::EvidenceDeliveryShape::OneSentence
        | crate::evidence_policy::EvidenceDeliveryShape::Summary => matches!(
            operation,
            crate::evidence_policy::EvidenceOperation::Inspect
                | crate::evidence_policy::EvidenceOperation::List
                | crate::evidence_policy::EvidenceOperation::Count
                | crate::evidence_policy::EvidenceOperation::Run
        ),
    }
}

fn route_path_locator_allows_observed_listing(route: &crate::IntentOutputContract) -> bool {
    !route.delivery_required
        && route.locator_kind == crate::OutputLocatorKind::Path
        && (route.semantic_kind_is_unclassified()
            || route.semantic_kind_is(crate::OutputSemanticKind::ExistenceWithPath))
}

fn route_allows_prior_step_error_observed_replacement(route: &crate::IntentOutputContract) -> bool {
    if route_path_locator_allows_observed_listing(route) {
        return true;
    }
    if route.response_shape == crate::OutputResponseShape::Scalar {
        return true;
    }
    route.semantic_kind_is_any(&[
        crate::OutputSemanticKind::FilePaths,
        crate::OutputSemanticKind::ExistenceWithPath,
    ])
}

fn delivery_has_planned_content_beyond_observed_answer(delivery: &str, observed: &str) -> bool {
    let delivery = delivery.trim();
    let observed = observed.trim();
    if delivery.is_empty() || observed.is_empty() || delivery == observed {
        return false;
    }
    if !delivery.contains(observed) {
        return false;
    }
    delivery
        .replacen(observed, "", 1)
        .chars()
        .any(|ch| !ch.is_whitespace())
}

pub(super) fn should_keep_planned_delivery_over_observed_answer(
    route: &crate::IntentOutputContract,
    delivery: &str,
    observed: &str,
) -> bool {
    if crate::finalize::is_execution_summary_message(delivery) {
        return false;
    }
    if route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput) {
        return false;
    }
    let scalar_model_language_verdict = route.response_shape == crate::OutputResponseShape::Scalar
        && route.semantic_kind_is(crate::OutputSemanticKind::ExistenceWithPath);
    if route.delivery_required && !scalar_model_language_verdict {
        return false;
    }
    if route_allows_model_language_final_answer(route)
        && (!output_contract_requests_exact_delivery(route) || scalar_model_language_verdict)
        && (!matches!(
            route.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        ) || scalar_model_language_verdict)
        && planned_delivery_is_publishable_model_language_answer(delivery)
    {
        return true;
    }
    if matches!(
        route.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        return false;
    }
    if route.requires_content_evidence
        && !route.delivery_required
        && !output_contract_requests_exact_delivery(route)
        && planned_delivery_is_publishable_model_language_answer(delivery)
        && delivery_is_structurally_richer_than_observed_projection(delivery, observed)
    {
        return true;
    }
    let planned_delivery_contains_more_than_observed =
        delivery_has_planned_content_beyond_observed_answer(delivery, observed);
    if !planned_delivery_contains_more_than_observed {
        return false;
    }
    if !output_contract_requests_exact_delivery(route) {
        return true;
    }
    false
}

fn delivery_is_structurally_richer_than_observed_projection(
    delivery: &str,
    observed: &str,
) -> bool {
    let delivery = delivery.trim();
    let observed = observed.trim();
    if delivery.is_empty() || observed.is_empty() || delivery == observed {
        return false;
    }
    let delivery_lines = delivery
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let delivery_chars = delivery.chars().count();
    let observed_chars = observed.chars().count();
    delivery_lines > 1 && delivery_chars > observed_chars.saturating_add(32)
}

fn should_keep_publishable_summary_over_raw_command_projection(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    delivery: &str,
    observed: &str,
) -> bool {
    if !matches!(
        crate::evidence_policy::final_answer_shape_for_output_contract(route),
        Some(
            crate::evidence_policy::FinalAnswerShape::SummaryWithEvidence
                | crate::evidence_policy::FinalAnswerShape::RawOutputOrShortSummary
        )
    ) {
        return false;
    }
    if route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && !publishable_summary_has_multi_source_observation(loop_state)
    {
        return false;
    }
    let delivery = delivery.trim();
    let observed = observed.trim();
    if delivery.is_empty()
        || observed.is_empty()
        || !planned_delivery_is_publishable_model_language_answer(delivery)
    {
        return false;
    }
    let nonempty_lines = delivery
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let token_count = delivery.split_whitespace().count();
    let delivery_chars = delivery.chars().count();
    let observed_chars = observed.chars().count();
    (nonempty_lines > 1 || token_count >= 8 || delivery_chars >= 64)
        && delivery_chars > observed_chars.saturating_add(16)
}

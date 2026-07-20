#[path = "loop_reply_matrix_shape_list_projection.rs"]
mod list_projection;
pub(super) use list_projection::generic_observed_machine_projection_answer;
#[cfg(test)]
pub(super) use list_projection::matrix_strict_list_observed_answer;
use list_projection::{
    matrix_observed_answer_candidate_for_shape, route_requests_exact_name_list,
    selected_name_list_prefers_observed_projection, stale_file_token_delivery_bounded_read_answer,
    stale_file_token_delivery_listing_answer,
};

use tracing::info;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::finalize::build_from_loop_state;
use crate::{AppState, ClaimedTask};

use super::{
    direct_file_token_from_observed_auto_locator_filename,
    direct_file_token_from_observed_find_entries, direct_file_token_from_observed_inventory,
    direct_generated_file_path_report_from_dry_run_payload,
    direct_generated_file_path_report_from_written_path, direct_path_from_active_bound_inventory,
    direct_scalar_observed_answer, direct_scalar_path_candidate_list_from_observed_outputs,
    direct_structured_observed_answer_allowing_implicit_metadata_path_facts,
    final_answer_text_from_delivery, inventory_ranked_size_list_answer,
    latest_bounded_read_range_answer_from_loop, latest_plan_requested_synthesis,
    log_deterministic_delivery_record,
    successful_content_observation_should_precede_status_summary,
};

fn evidence_policy_final_answer_shape_class(
    route: &crate::IntentOutputContract,
) -> Option<crate::evidence_policy::FinalAnswerShapeClass> {
    if route_requests_exact_name_list(route) {
        return Some(crate::evidence_policy::FinalAnswerShapeClass::StrictList);
    }
    crate::evidence_policy::final_answer_shape_for_output_contract(route).map(|shape| shape.class())
}

pub(super) fn route_requires_evidence_policy_deterministic_final_answer(
    route: &crate::IntentOutputContract,
) -> bool {
    evidence_policy_final_answer_shape_class(route)
        .is_some_and(|class| !class.allows_model_language())
}

pub(super) fn agent_context_allows_observed_output_language_fallback(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_none_or(|route| !route_requires_evidence_policy_deterministic_final_answer(route))
}

pub(super) fn should_try_observed_output_language_fallback(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(crate::agent_engine::observed_output::route_requires_synthesized_delivery)
        || agent_context_allows_observed_output_language_fallback(agent_run_context)
        || latest_plan_requested_synthesis(loop_state)
        || successful_content_observation_should_precede_status_summary(
            agent_run_context,
            loop_state,
        )
}

#[cfg(test)]
pub(super) fn route_has_evidence_policy_final_shape(route: &crate::IntentOutputContract) -> bool {
    evidence_policy_final_answer_shape_class(route).is_some()
}

pub(super) fn route_requires_observed_output_projection(
    route: &crate::IntentOutputContract,
) -> bool {
    if matches!(
        evidence_policy_final_answer_shape_class(route),
        Some(
            crate::evidence_policy::FinalAnswerShapeClass::DeliveryArtifact
                | crate::evidence_policy::FinalAnswerShapeClass::SinglePath
                | crate::evidence_policy::FinalAnswerShapeClass::StrictList
        )
    ) {
        return true;
    }
    route_requests_exact_name_list(route)
}

pub(super) fn evidence_policy_candidate_satisfies_final_shape(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    route: &crate::IntentOutputContract,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    let delivery_messages = vec![candidate.to_string()];
    let journal = build_from_loop_state(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        crate::task_journal::delivery_payload_consistent(candidate, &delivery_messages),
        candidate,
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    let answer_contract = crate::answer_verifier::AnswerContract::new("", route.clone());
    crate::answer_verifier::structurally_satisfies_answer_contract(
        &answer_contract,
        &journal,
        candidate,
    )
}

pub(super) fn synthetic_task_for_evidence_policy_shape_check(task_id: &str) -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: task_id.to_string(),
        user_id: 0,
        chat_id: 0,
        user_key: None,
        channel: "finalize".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

pub(super) fn current_synthesis_satisfies_evidence_policy_shape(
    task_id: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    route: &crate::IntentOutputContract,
    delivery_messages: &[String],
) -> bool {
    if !route_requires_evidence_policy_deterministic_final_answer(route) {
        return true;
    }
    let Some(message) = delivery_messages.last() else {
        return false;
    };
    if selected_name_list_prefers_observed_projection(route, loop_state) {
        return false;
    }
    let task = synthetic_task_for_evidence_policy_shape_check(task_id);
    evidence_policy_candidate_satisfies_final_shape(
        &task,
        "",
        loop_state,
        agent_run_context,
        finalizer_summary,
        route,
        message,
    )
}

pub(super) fn matrix_observed_shape_summary(
    loop_state: &LoopState,
) -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    }
}

pub(super) fn replace_delivery_with_matrix_observed_shape_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if loop_state.pending_user_input_required {
        return false;
    }
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if !route_requires_evidence_policy_deterministic_final_answer(route) {
        return false;
    }
    if let Some((candidate, summary)) =
        direct_path_from_active_bound_inventory(loop_state, agent_run_context)
    {
        let answer = candidate.trim().to_string();
        if answer.is_empty() {
            return false;
        }
        if final_answer_text_from_delivery(delivery_messages).trim() == answer {
            *finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer);
            return true;
        }
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        log_deterministic_delivery_record(
            &task.task_id,
            "matrix_replace_active_bound_inventory_path",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    let Some(shape_class) = evidence_policy_final_answer_shape_class(route) else {
        return false;
    };
    let current_answer = final_answer_text_from_delivery(delivery_messages);
    if let Some((candidate, summary)) =
        stale_file_token_delivery_listing_answer(route, loop_state, delivery_messages)
    {
        let answer = candidate.trim().to_string();
        if answer.is_empty() {
            return false;
        }
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        log_deterministic_delivery_record(
            &task.task_id,
            "matrix_replace_stale_file_token_with_listing",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if let Some((candidate, summary)) =
        stale_file_token_delivery_bounded_read_answer(route, loop_state, delivery_messages)
    {
        let answer = candidate.trim().to_string();
        if answer.is_empty() {
            return false;
        }
        delivery_messages.clear();
        delivery_messages.push(answer.clone());
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        log_deterministic_delivery_record(
            &task.task_id,
            "matrix_replace_stale_file_token_with_bounded_read",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
        return true;
    }
    if !current_answer.trim().is_empty()
        && !selected_name_list_prefers_observed_projection(route, loop_state)
        && evidence_policy_candidate_satisfies_final_shape(
            task,
            user_text,
            loop_state,
            agent_run_context,
            finalizer_summary.clone(),
            route,
            &current_answer,
        )
    {
        return false;
    }
    let Some((candidate, summary)) = matrix_observed_answer_candidate_for_shape(
        state,
        loop_state,
        agent_run_context,
        shape_class,
    ) else {
        return false;
    };
    if !selected_name_list_prefers_observed_projection(route, loop_state)
        && !evidence_policy_candidate_satisfies_final_shape(
            task,
            user_text,
            loop_state,
            agent_run_context,
            Some(summary.clone()),
            route,
            &candidate,
        )
    {
        return false;
    }

    let answer = candidate.trim().to_string();
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    info!(
        "delivery matrix_shape_from_observed task_id={} shape_class={} answer={}",
        task.task_id,
        shape_class.as_str(),
        crate::truncate_for_log(&candidate)
    );
    log_deterministic_delivery_record(
        &task.task_id,
        "matrix_shape_from_observed",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn finalizer_summary_requires_matrix_observed_replacement(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(summary) = summary else {
        return false;
    };
    summary.needs_clarify == Some(true)
        || !summary.contract_ok
        || summary.format_ok == Some(false)
        || summary.grounded_ok == Some(false)
}

pub(crate) fn deterministic_matrix_observed_shape_answer(
    state: &AppState,
    _task: &ClaimedTask,
    _user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract())?;
    if !route_requires_evidence_policy_deterministic_final_answer(route) {
        return None;
    }
    let shape_class = evidence_policy_final_answer_shape_class(route)?;
    let (candidate, summary) = matrix_observed_answer_candidate_for_shape(
        state,
        loop_state,
        agent_run_context,
        shape_class,
    )?;
    let candidate = candidate.trim().to_string();
    if candidate.is_empty() {
        return None;
    }
    Some((candidate, summary))
}

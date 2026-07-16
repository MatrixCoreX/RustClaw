use tracing::info;

use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::ClaimedTask;

pub(super) fn replace_delivery_with_service_status_observed_answer(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let Some(answer) =
        super::service_status::service_status_system_basic_info_answer(route, loop_state)
    else {
        return false;
    };
    if answer.trim().is_empty() {
        return false;
    }
    let summary = crate::task_journal::TaskJournalFinalizerSummary {
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
    };
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer.trim())
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = Some(answer.clone());
    append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
    *finalizer_summary = Some(summary);
    super::delivery_record::log_deterministic_delivery_record(
        &task.task_id,
        "service_status_observed_fields",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn replace_raw_passthrough_delivery_with_publishable_synthesis(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if !super::delivery_backfill::route_expects_synthesis_over_raw_observation(route) {
        return false;
    }
    let Some(synthesis) = super::delivery_backfill::valid_publishable_synthesis_output(loop_state)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
    else {
        return false;
    };
    if crate::finalize::looks_like_planner_artifact(&synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(&synthesis)
        || crate::finalize::parse_delivery_token(&synthesis).is_some()
    {
        return false;
    }
    let has_raw_passthrough = delivery_messages.iter().any(|message| {
        let candidate = message.trim();
        !candidate.is_empty()
            && candidate != synthesis
            && (crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
                candidate, loop_state,
            ) || super::delivery_backfill::candidate_matches_successful_external_observation(
                loop_state, candidate,
            ))
    });
    if !has_raw_passthrough {
        return false;
    }
    info!(
        "final_result_replace_raw_passthrough_delivery_with_synthesis task_id={} synthesis={}",
        task.task_id,
        crate::truncate_for_log(&synthesis)
    );
    delivery_messages.clear();
    delivery_messages.push(synthesis.clone());
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        synthesis.clone(),
    );
    loop_state.last_user_visible_respond = Some(synthesis);
    super::delivery_record::log_deterministic_delivery_record(
        &task.task_id,
        "final_result_replace_raw_passthrough_delivery_with_synthesis",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn prefer_latest_synthesis_for_compound_observation_delivery(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let contract = route.clone();
    if contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
    {
        return false;
    }
    if super::raw_command::output_contract_requests_exact_delivery(route) {
        return false;
    }
    if !contract.requires_content_evidence
        && !super::delivery_backfill::route_expects_synthesis_over_raw_observation(route)
        && !route_allows_grounded_compound_terminal_delivery(route, loop_state)
    {
        return false;
    }
    let current = super::delivery_text::final_answer_text_from_delivery(delivery_messages)
        .trim()
        .to_string();
    if super::route_helpers::delivery_message_is_json_container(&current) {
        return false;
    }
    let Some(synthesis) =
        super::delivery_backfill::latest_publishable_synthesis_step_output(loop_state)
            .or_else(|| super::delivery_backfill::latest_contractual_synthesis_output(loop_state))
            .or_else(|| latest_publishable_terminal_language_output(loop_state))
            .map(str::trim)
            .filter(|text| {
                super::language_closeout::planned_delivery_is_publishable_model_language_answer(
                    text,
                ) || structured_compound_synthesis_can_replace_current_delivery(
                    route, loop_state, &current, text,
                )
            })
            .map(str::to_string)
    else {
        return false;
    };
    if current.is_empty() || current == synthesis {
        return false;
    }
    let synthesis_is_structured_json =
        super::route_helpers::delivery_message_is_json_container(&synthesis);
    let current_chars = current.chars().count();
    let synthesis_chars = synthesis.chars().count();
    if !synthesis_is_structured_json
        && synthesis_chars <= current_chars + 80
        && synthesis_chars.saturating_mul(4) <= current_chars.saturating_mul(5)
    {
        return false;
    }

    delivery_messages.clear();
    delivery_messages.push(synthesis.clone());
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
    super::delivery_record::log_deterministic_delivery_record(
        &task.task_id,
        "compound_observation_latest_synthesis",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn prefer_content_evidence_synthesis_for_final_delivery(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let Some(synthesis) = super::delivery_backfill::valid_publishable_synthesis_output(loop_state)
        .or_else(|| super::delivery_backfill::latest_publishable_respond_step_output(loop_state))
        .or_else(|| super::delivery_backfill::latest_contractual_synthesis_output(loop_state))
        .map(str::trim)
        .filter(|text| {
            super::contract_enforce::route_prefers_content_evidence_synthesis(route, text)
        })
        .map(str::to_string)
    else {
        return false;
    };
    let current = super::delivery_text::final_answer_text_from_delivery(delivery_messages)
        .trim()
        .to_string();
    if current == synthesis {
        return false;
    }
    delivery_messages.clear();
    append_delivery_message(task.task_id.as_str(), delivery_messages, synthesis.clone());
    loop_state.delivery_messages = delivery_messages.clone();
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
    super::delivery_record::log_deterministic_delivery_record(
        &task.task_id,
        "content_evidence_final_keep_publishable_synthesis",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn route_allows_grounded_compound_terminal_delivery(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    let contract = route.clone();
    if contract.delivery_required
        || !matches!(contract.response_shape, crate::OutputResponseShape::Free)
        || super::raw_command::output_contract_requests_exact_delivery(route)
    {
        return false;
    }
    let Some(shape) = crate::evidence_policy::final_answer_shape_for_output_contract(route) else {
        return false;
    };
    if !matches!(
        shape.class(),
        crate::evidence_policy::FinalAnswerShapeClass::GroundedSummary
            | crate::evidence_policy::FinalAnswerShapeClass::Freeform
    ) {
        return false;
    }
    successful_non_terminal_observation_count(loop_state) >= 2
}

pub(super) fn structured_compound_synthesis_can_replace_current_delivery(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    current: &str,
    synthesis: &str,
) -> bool {
    let current = current.trim();
    let synthesis = synthesis.trim();
    let agent_hook_policy_surface = agent_hook_policy_surface_payload_is_publishable(synthesis);
    if current.is_empty()
        || synthesis.is_empty()
        || super::route_helpers::delivery_message_is_json_container(current)
        || !super::route_helpers::delivery_message_is_json_container(synthesis)
        || crate::finalize::looks_like_planner_artifact(synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(synthesis)
        || (crate::finalize::is_execution_summary_message(synthesis) && !agent_hook_policy_surface)
        || super::raw_command::output_contract_requests_exact_delivery(route)
        || route.delivery_required
    {
        return false;
    }
    if matches!(
        route.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    ) {
        return false;
    }
    successful_non_terminal_observation_count(loop_state) >= 2
}

fn successful_non_terminal_observation_count(loop_state: &LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think" | "answer_verifier"
                )
        })
        .count()
}

fn agent_hook_policy_surface_payload_is_publishable(synthesis: &str) -> bool {
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(synthesis.trim()) else {
        return false;
    };
    payload
        .pointer("/owner_layer")
        .and_then(serde_json::Value::as_str)
        == Some("agent_hooks")
        && payload
            .pointer("/stage")
            .and_then(serde_json::Value::as_str)
            == Some("pre_tool_use")
        && payload
            .pointer("/reason_code")
            .and_then(serde_json::Value::as_str)
            == Some("agent_hooks_pre_tool_use_policy_surface")
        && payload
            .pointer("/decision_tokens")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tokens| {
                ["allow", "deny", "require_confirmation", "background_wait"]
                    .iter()
                    .all(|expected| tokens.iter().any(|token| token.as_str() == Some(*expected)))
            })
}

fn latest_publishable_terminal_language_output(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter(|step| matches!(step.skill.as_str(), "synthesize_answer" | "respond"))
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .find(|output| {
            !output.is_empty()
                && super::language_closeout::planned_delivery_is_publishable_model_language_answer(
                    output,
                )
                && !crate::finalize::is_execution_summary_message(output)
        })
}

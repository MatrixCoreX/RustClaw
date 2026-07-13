use crate::agent_engine::{AgentRunContext, LoopState};
use crate::finalize::build_from_loop_state as build_loop_journal;
use crate::{AppState, AskReply, ClaimedTask};

use super::{
    deterministic_execution_failed_step_answer, deterministic_matrix_observed_shape_answer,
    deterministic_missing_observed_target_answer, deterministic_observed_execution_status_answer,
    deterministic_observed_execution_status_summary,
    deterministic_structured_container_summary_answer, direct_config_edit_observed_answer,
    direct_current_workspace_top_level_dirs_overview_answer, direct_db_basic_observed_answer,
    direct_generated_file_path_report_from_dry_run_payload,
    direct_quantity_comparison_from_compare_paths, direct_rustclaw_config_risk_answer,
    output_text_from_execution_result, planned_delivery_identifies_failed_observed_step,
    preferred_route_clarify_question, route_prefers_language_rendered_execution_failed_step,
    route_resolved_intent, route_structured_clarify_context,
};

pub(super) async fn pending_confirmation_resume_payload(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
) -> Option<(String, serde_json::Value)> {
    let round = loop_state.round_traces.last()?;
    let verify = round.verify_result.as_ref()?;
    if !verify_summary_requires_resume_confirmation(verify) {
        return None;
    }
    let plan = round.plan_result.as_ref()?;
    let detail = verify
        .issues
        .iter()
        .find(|issue| issue.kind == crate::verifier::VerifyIssueKind::ConfirmationRequired)
        .map(|issue| issue.detail.as_str())
        .unwrap_or("current plan requires explicit confirmation");
    Some(
        crate::agent_engine::build_confirmation_required_resume_context(
            state,
            task,
            &plan.steps,
            user_text,
            &round.goal,
            &loop_state.subtask_results,
            &loop_state.delivery_messages,
            detail,
        )
        .await,
    )
}

pub(super) fn verify_summary_requires_resume_confirmation(
    verify: &crate::task_journal::TaskJournalVerifySummary,
) -> bool {
    verify.mode == crate::verifier::VerifyMode::Enforce
        && verify.approved
        && verify.needs_confirmation
}

pub(super) fn finalizer_requires_clarify(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
    requires_content_evidence: bool,
    has_authoritative_delivery: bool,
) -> bool {
    if requires_content_evidence {
        if has_authoritative_delivery {
            return false;
        }
        return !matches!(
            summary.and_then(|summary| summary.disposition),
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        );
    }
    false
}

pub(super) fn build_finalizer_clarify_reason(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> String {
    let Some(summary) = summary else {
        return "finalizer could not confirm a reliable final answer from the observed execution result"
            .to_string();
    };
    let mut parts = Vec::new();
    if let Some(stage) = summary
        .stage
        .map(crate::task_journal::TaskJournalFinalizerStage::as_str)
    {
        parts.push(format!("stage={stage}"));
    }
    if let Some(disposition) = summary
        .disposition
        .map(crate::finalize::FinalizerDisposition::as_str)
        .filter(|v| !v.trim().is_empty())
    {
        parts.push(format!("disposition={disposition}"));
    }
    if let Some(fallback) = summary
        .fallback
        .map(crate::task_journal::TaskJournalFinalizerFallback::as_str)
    {
        parts.push(format!("fallback={fallback}"));
    }
    if let Some(value) = summary.completion_ok {
        parts.push(format!("completion_ok={value}"));
    }
    if let Some(value) = summary.grounded_ok {
        parts.push(format!("grounded_ok={value}"));
    }
    if let Some(value) = summary.format_ok {
        parts.push(format!("format_ok={value}"));
    }
    if let Some(value) = summary.needs_clarify {
        parts.push(format!("needs_clarify={value}"));
    }
    if parts.is_empty() {
        "finalizer could not confirm a reliable final answer from the observed execution result"
            .to_string()
    } else {
        format!(
            "finalizer could not confirm a reliable final answer from the observed execution result; {}",
            parts.join(", ")
        )
    }
}

pub(super) fn build_missing_delivery_clarify_reason(
    summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> String {
    match summary {
        Some(summary) => format!(
            "no publishable final answer was produced; {}",
            build_finalizer_clarify_reason(Some(summary))
        ),
        None => "no publishable final answer was produced from the execution result".to_string(),
    }
}

pub(super) fn build_pending_user_input_clarify_reason(
    loop_state: &crate::agent_engine::LoopState,
    fallback: String,
) -> String {
    if !loop_state.pending_user_input_required {
        return fallback;
    }
    let mut parts = Vec::new();
    for key in [
        "agent_loop.terminal_intent",
        "agent_loop.clarify_reason_code",
        "agent_loop.missing_slot",
        "agent_loop.message_key",
        "agent_loop.field_path",
        "agent_loop.locator_kind",
    ] {
        if let Some(value) = loop_state
            .output_vars
            .get(key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!("{key}={value}"));
        }
    }
    if parts.is_empty() {
        fallback
    } else {
        parts.join("; ")
    }
}

fn observed_execution_facts_for_missing_delivery(
    loop_state: &crate::agent_engine::LoopState,
    clarify_reason: &str,
) -> Vec<String> {
    let mut facts = vec![format!(
        "finalizer_reason: {}",
        crate::truncate_for_agent_trace(clarify_reason)
    )];
    let mut steps = loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            ) && output_text_from_execution_result(step).is_some()
        })
        .rev()
        .take(4)
        .collect::<Vec<_>>();
    steps.reverse();
    for step in steps {
        let mut parts = vec![
            format!("skill={}", step.skill.trim()),
            format!("status={}", step.status.as_str()),
        ];
        if let Some(output) = output_text_from_execution_result(step) {
            parts.push(format!(
                "observed_output={}",
                crate::truncate_for_agent_trace(&output)
            ));
        }
        facts.push(format!("observed_step: {}", parts.join(", ")));
    }
    facts
}

async fn observed_answer_reply_from_missing_delivery_context(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<AskReply> {
    if !crate::agent_engine::observed_output::has_observed_answer_candidates(loop_state) {
        return None;
    }
    let (answer, summary) =
        match crate::agent_engine::observed_output::try_synthesize_answer_from_observed_output(
            state,
            task,
            user_text,
            loop_state,
            agent_run_context,
        )
        .await
        {
            Ok(Some(outcome)) => outcome,
            Ok(None) => return None,
            Err(err) => {
                tracing::warn!(
                    "missing_delivery_observed_answer_retry_unavailable task_id={} err={}",
                    task.task_id,
                    err
                );
                return None;
            }
        };
    let answer = answer.trim();
    if answer.is_empty()
        || !matches!(
            summary.disposition,
            Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        )
        || summary.completion_ok == Some(false)
    {
        return None;
    }
    let delivery_messages = vec![answer.to_string()];
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(answer, &delivery_messages);
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        Some(summary),
        delivery_consistent,
        answer,
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    Some(
        AskReply::non_llm(answer.to_string())
            .with_messages(delivery_messages)
            .with_task_journal(journal),
    )
}

async fn missing_delivery_after_observation_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    clarify_reason: &str,
    observed_contract_evidence_complete: bool,
) -> String {
    let prefer_language_rendered_failed_step =
        route_prefers_language_rendered_execution_failed_step(agent_run_context);
    if !prefer_language_rendered_failed_step {
        if let Some(answer) = deterministic_execution_failed_step_answer(
            state,
            user_text,
            loop_state,
            agent_run_context,
        ) {
            return answer;
        }
    }
    if !prefer_language_rendered_failed_step {
        if let Some(answer) =
            deterministic_observed_execution_status_answer(state, user_text, loop_state)
        {
            return answer;
        }
    }
    if let Some((answer, _summary)) =
        direct_config_edit_observed_answer(state, user_text, loop_state)
    {
        return answer;
    }
    if let Some((answer, _summary)) =
        direct_rustclaw_config_risk_answer(state, user_text, loop_state)
    {
        return answer;
    }
    if let Some((answer, _summary)) = direct_quantity_comparison_from_compare_paths(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) {
        return answer;
    }
    if let Some((answer, _summary)) = direct_current_workspace_top_level_dirs_overview_answer(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) {
        return answer;
    }
    if let Some(answer) = deterministic_structured_container_summary_answer(
        state,
        user_text,
        loop_state,
        agent_run_context,
    ) {
        return answer;
    }
    if let Some((answer, _summary)) =
        direct_db_basic_observed_answer(state, user_text, loop_state, agent_run_context)
    {
        return answer;
    }
    if let Some((answer, _summary)) =
        direct_generated_file_path_report_from_dry_run_payload(loop_state, agent_run_context)
    {
        return answer;
    }
    if let Some((answer, _summary)) = deterministic_matrix_observed_shape_answer(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
    ) {
        return answer;
    }
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let observed_facts = observed_execution_facts_for_missing_delivery(loop_state, clarify_reason);
    let contract = if observed_contract_evidence_complete {
        crate::fallback::UserResponseContract {
            kind: crate::fallback::UserResponseKind::FinalAnswer,
            reason_code: "observed_execution_contract_evidence_complete".to_string(),
            missing_slots: Vec::new(),
            observed_facts,
            policy_boundary: vec![
                "task_success_claim_allowed=true".to_string(),
                "answer_only_from_observed_facts=true".to_string(),
                "ask_item_selection_when_outputs_attached=false".to_string(),
            ],
            original_user_request: user_text.trim().to_string(),
            resolved_user_intent: route_resolved_intent(agent_run_context),
            response_shape: "brief_answer_from_observed_facts".to_string(),
            language_hint,
        }
    } else {
        crate::fallback::UserResponseContract::tool_failure(
            "final_answer_missing_after_observed_execution",
            user_text,
            &route_resolved_intent(agent_run_context),
            observed_facts,
            vec![
                "task_success_claim_allowed=false".to_string(),
                "ask_item_selection_when_outputs_attached=false".to_string(),
                "response_scope=observed_blocker_or_incomplete_result".to_string(),
                "next_step_policy=only_if_observed_facts_do_not_answer_request".to_string(),
            ],
            "brief_failure_with_next_step",
            &language_hint,
        )
    };
    crate::fallback::compose_user_response_from_contract(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
    )
    .await
}

pub(super) async fn observed_execution_without_publishable_delivery_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    clarify_reason: &str,
) -> Option<AskReply> {
    let status_summary = || deterministic_observed_execution_status_summary(loop_state);
    let prefer_language_rendered_failed_step =
        route_prefers_language_rendered_execution_failed_step(agent_run_context);
    let deterministic_failed_step_answer = if prefer_language_rendered_failed_step {
        None
    } else {
        deterministic_execution_failed_step_answer(state, user_text, loop_state, agent_run_context)
            .map(|answer| (answer, status_summary()))
    };
    let deterministic_answer = deterministic_failed_step_answer
        .or_else(|| {
            (!prefer_language_rendered_failed_step)
                .then(|| {
                    deterministic_observed_execution_status_answer(state, user_text, loop_state)
                        .map(|answer| (answer, status_summary()))
                })
                .flatten()
        })
        .or_else(|| direct_config_edit_observed_answer(state, user_text, loop_state))
        .or_else(|| direct_rustclaw_config_risk_answer(state, user_text, loop_state))
        .or_else(|| {
            direct_quantity_comparison_from_compare_paths(
                state,
                user_text,
                loop_state,
                agent_run_context,
            )
        })
        .or_else(|| {
            direct_current_workspace_top_level_dirs_overview_answer(
                state,
                user_text,
                loop_state,
                agent_run_context,
            )
        })
        .or_else(|| {
            deterministic_structured_container_summary_answer(
                state,
                user_text,
                loop_state,
                agent_run_context,
            )
            .map(|answer| (answer, status_summary()))
        })
        .or_else(|| {
            direct_db_basic_observed_answer(state, user_text, loop_state, agent_run_context)
        })
        .or_else(|| {
            direct_generated_file_path_report_from_dry_run_payload(loop_state, agent_run_context)
        })
        .or_else(|| {
            deterministic_matrix_observed_shape_answer(
                state,
                task,
                user_text,
                loop_state,
                agent_run_context,
            )
        })
        .or_else(|| {
            deterministic_missing_observed_target_answer(
                state,
                user_text,
                loop_state,
                agent_run_context,
            )
            .map(|answer| (answer, status_summary()))
        });
    let has_deterministic_answer = deterministic_answer.is_some();
    if !has_deterministic_answer
        && finalizer_summary
            .as_ref()
            .and_then(|summary| summary.needs_clarify)
            .unwrap_or(false)
    {
        let structured_clarify_context = route_structured_clarify_context(agent_run_context);
        let clarify = crate::finalize::render_clarify_question(
            state,
            task,
            crate::finalize::ClarifyRenderRequest {
                user_request: user_text,
                resolver_reason: clarify_reason,
                candidate_context: structured_clarify_context.as_deref(),
                preferred_question: preferred_route_clarify_question(agent_run_context),
                policy: crate::intent_router::ClarifyQuestionPolicy::SafeFallback,
                fallback_source: crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
            },
        )
        .await;
        let delivery_messages = vec![clarify.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&clarify, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &clarify,
            crate::task_journal::TaskJournalFinalStatus::Clarify,
        );
        return Some(
            AskReply::non_llm(clarify.clone())
                .with_messages(delivery_messages)
                .with_task_journal(journal),
        );
    }
    if !has_deterministic_answer {
        if let Some(reply) = observed_answer_reply_from_missing_delivery_context(
            state,
            task,
            user_text,
            loop_state,
            agent_run_context,
        )
        .await
        {
            return Some(reply);
        }
    }
    let observed_contract_evidence_complete = observed_execution_has_complete_contract_evidence(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary.as_ref(),
    );
    let message = missing_delivery_after_observation_message(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
        clarify_reason,
        observed_contract_evidence_complete,
    )
    .await;
    let delivery_messages = vec![message.clone()];
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
    let has_deterministic_answer = deterministic_answer.is_some();
    let language_rendered_failed_step_summary =
        language_rendered_failed_step_finalizer_summary(agent_run_context, loop_state, &message);
    let has_language_rendered_failed_step_answer = language_rendered_failed_step_summary.is_some();
    let mut finalizer_summary = finalizer_summary
        .or_else(|| {
            deterministic_answer
                .as_ref()
                .map(|(_, summary)| summary.clone())
        })
        .or(language_rendered_failed_step_summary);
    let has_observed_language_answer = observed_delivery_has_complete_contract_evidence(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary.as_ref(),
        &message,
    );
    if has_observed_language_answer {
        finalizer_summary = Some(promote_observed_language_delivery_summary(
            finalizer_summary,
            loop_state,
        ));
    }
    let (final_status, should_fail_task) = observed_execution_without_publishable_delivery_outcome(
        has_deterministic_answer
            || has_language_rendered_failed_step_answer
            || has_observed_language_answer,
        finalizer_summary.as_ref(),
    );
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_consistent,
        &message,
        final_status,
    );
    let reply = AskReply::non_llm(message.clone())
        .with_messages(delivery_messages)
        .with_task_journal(journal);
    Some(if should_fail_task {
        reply.with_failure(message)
    } else {
        reply
    })
}

pub(super) fn observed_synthesis_unavailable_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    err: &str,
) -> AskReply {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let context_hint = format!(
        "observed_finalizer={}",
        crate::truncate_for_agent_trace(err)
    );
    let message = crate::fallback::render_clarify_fallback_with_language_hint(
        state,
        &task.task_id,
        crate::fallback::ClarifyFallbackSource::LlmUnavailable,
        Some(&context_hint),
        &language_hint,
    );
    let delivery_messages = vec![message.clone()];
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&message, &delivery_messages);
    let finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
        parsed: false,
        contract_ok: false,
        completion_ok: Some(false),
        grounded_ok: None,
        format_ok: None,
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_consistent,
        &message,
        crate::task_journal::TaskJournalFinalStatus::Failure,
    );
    AskReply::non_llm(message.clone())
        .with_messages(delivery_messages)
        .with_task_journal(journal)
        .with_failure(message)
}

pub(super) fn language_rendered_failed_step_finalizer_summary(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
    message: &str,
) -> Option<crate::task_journal::TaskJournalFinalizerSummary> {
    (route_prefers_language_rendered_execution_failed_step(agent_run_context)
        && planned_delivery_identifies_failed_observed_step(message, loop_state))
    .then(|| deterministic_observed_execution_status_summary(loop_state))
}

pub(super) fn observed_delivery_has_complete_contract_evidence(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
    message: &str,
) -> bool {
    !message.trim().is_empty()
        && observed_execution_has_complete_contract_evidence(
            task,
            user_text,
            loop_state,
            agent_run_context,
            finalizer_summary,
        )
}

fn observed_execution_has_complete_contract_evidence(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if route.output_contract_is_unclassified()
        && !route.output_contract.delivery_required
        && !route.wants_file_delivery
    {
        return false;
    }
    if route.needs_clarify
        || finalizer_summary
            .and_then(|summary| summary.needs_clarify)
            .unwrap_or(false)
    {
        return false;
    }
    if !loop_state.executed_step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer" | "answer_verifier"
            )
            && output_text_from_execution_result(step).is_some()
    }) {
        return false;
    }
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary.cloned(),
        true,
        "",
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    let coverage = crate::task_journal::evidence_coverage_for_route(route, &journal);
    let has_contractual_evidence =
        !coverage.required_evidence.is_empty() || coverage.evidence_expression.is_some();
    has_contractual_evidence && coverage.is_complete()
}

pub(super) fn promote_observed_language_delivery_summary(
    base: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    loop_state: &crate::agent_engine::LoopState,
) -> crate::task_journal::TaskJournalFinalizerSummary {
    let mut summary = base.unwrap_or_default();
    summary.stage = Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric);
    summary.disposition = Some(crate::finalize::FinalizerDisposition::QualifiedCompletion);
    summary.contract_ok = true;
    summary.completion_ok = Some(true);
    summary.grounded_ok = Some(true);
    summary.format_ok = Some(true);
    summary.needs_clarify = Some(false);
    summary.confidence = Some(summary.confidence.unwrap_or(0.0).max(0.8));
    summary.used_evidence_ids_count = summary
        .used_evidence_ids_count
        .max(loop_state.executed_step_results.len());
    summary
}

pub(super) fn observed_execution_without_publishable_delivery_outcome(
    has_deterministic_answer: bool,
    finalizer_summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
) -> (crate::task_journal::TaskJournalFinalStatus, bool) {
    if has_deterministic_answer {
        return (crate::task_journal::TaskJournalFinalStatus::Success, false);
    }
    if finalizer_summary
        .and_then(|summary| summary.needs_clarify)
        .unwrap_or(false)
    {
        return (crate::task_journal::TaskJournalFinalStatus::Clarify, false);
    }
    (crate::task_journal::TaskJournalFinalStatus::Failure, true)
}

pub(super) fn successful_delivery_final_status(
    loop_state: &crate::agent_engine::LoopState,
    finalizer_summary: Option<&crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &[String],
) -> crate::task_journal::TaskJournalFinalStatus {
    if finalizer_summary
        .and_then(|summary| summary.needs_clarify)
        .unwrap_or(false)
    {
        return crate::task_journal::TaskJournalFinalStatus::Clarify;
    }
    let has_non_clarify_delivery = delivery_messages
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .any(|message| {
            !message.is_empty()
                && !super::clarify_envelope::delivery_has_terminal_clarify_machine_fields(message)
        });
    if loop_state.pending_user_input_required && !has_non_clarify_delivery {
        crate::task_journal::TaskJournalFinalStatus::Clarify
    } else {
        crate::task_journal::TaskJournalFinalStatus::Success
    }
}

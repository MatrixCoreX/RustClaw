use tracing::info;

use super::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

fn backfill_delivery_from_last_outputs(task: &ClaimedTask, loop_state: &mut LoopState) {
    if loop_state.delivery_messages.is_empty() {
        if let Some(ref last_respond) = loop_state.last_user_visible_respond {
            if !last_respond.trim().is_empty() {
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    last_respond.clone(),
                );
                info!(
                    "final_result_use_last_respond task_id={} (delivery was empty)",
                    task.task_id
                );
            }
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some(ref last_chat_output) = loop_state.last_publishable_chat_output {
            if !last_chat_output.trim().is_empty() {
                append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    last_chat_output.clone(),
                );
                info!(
                    "final_result_use_chat_output task_id={} (delivery was empty)",
                    task.task_id
                );
            }
        }
    }
}

fn route_requires_content_evidence(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.requires_content_evidence)
        .unwrap_or(false)
}

async fn discard_meta_respond_placeholder_for_content_evidence(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    requires_content_evidence: bool,
) {
    if !requires_content_evidence {
        return;
    }
    if !loop_state.has_tool_or_skill_output {
        return;
    }
    if loop_state.delivery_messages.len() != 1 {
        return;
    }
    let Some(last_respond) = loop_state.last_user_visible_respond.as_deref() else {
        return;
    };
    let delivery = loop_state.delivery_messages[0].trim();
    let respond = last_respond.trim();
    if delivery.is_empty() || respond.is_empty() || delivery != respond {
        return;
    }
    if !crate::semantic_judge::is_meta_respond_instruction(state, task, respond).await {
        return;
    }
    info!(
        "content_evidence_drop_meta_respond task_id={} text={}",
        task.task_id,
        crate::truncate_for_log(respond)
    );
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
}

fn direct_scalar_observed_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return None;
    }
    let answer = super::observed_output::extract_direct_scalar_from_generic_output(loop_state)?;
    if crate::finalizer::looks_like_planner_artifact(&answer)
        || crate::finalizer::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            ..Default::default()
        },
    ))
}

fn pending_confirmation_resume_payload(
    state: &AppState,
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
    Some(super::build_confirmation_required_resume_context(
        &plan.steps,
        user_text,
        &round.goal,
        &loop_state.subtask_results,
        &loop_state.delivery_messages,
        detail,
        &state.command_intent.default_locale,
    ))
}

fn verify_summary_requires_resume_confirmation(
    verify: &crate::task_journal::TaskJournalVerifySummary,
) -> bool {
    verify.mode == crate::verifier::VerifyMode::Enforce
        && verify.approved
        && verify.needs_confirmation
}

fn finalizer_requires_clarify(
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
            Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
        );
    }
    false
}

fn build_finalizer_clarify_reason(
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
        .map(crate::finalizer::FinalizerDisposition::as_str)
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

fn build_missing_delivery_clarify_reason(
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

fn build_loop_journal(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_consistent: bool,
    final_text: &str,
    final_status: crate::task_journal::TaskJournalFinalStatus,
) -> crate::task_journal::TaskJournal {
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", user_text);
    if let Some(ctx) = agent_run_context {
        if let Some(route_result) = ctx.route_result.as_ref() {
            journal.record_route_result(route_result);
        }
        if let Some(context_summary) = ctx.context_bundle_summary.as_deref() {
            journal.record_context_bundle_summary(context_summary.to_string());
        }
    }
    journal.rounds = loop_state.round_traces.clone();
    for step in &loop_state.executed_step_results {
        journal.push_step_result(step);
    }
    if let Some(summary) = finalizer_summary {
        journal.record_finalizer_summary(summary);
    } else {
        journal.record_used_evidence_ids_count(0);
    }
    journal.record_delivery_consistent(delivery_consistent);
    journal.record_final_answer(final_text.to_string());
    journal.record_final_status(final_status);
    journal
}

pub(super) async fn finalize_loop_reply(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    mut loop_state: LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<AskReply, String> {
    backfill_delivery_from_last_outputs(task, &mut loop_state);

    if let Some((user_error, resume_context)) =
        pending_confirmation_resume_payload(state, user_text, &loop_state)
    {
        let delivery_messages = vec![user_error.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&user_error, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            None,
            delivery_consistent,
            &user_error,
            crate::task_journal::TaskJournalFinalStatus::ResumeFailure,
        );
        return Ok(AskReply::non_llm(user_error.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal)
            .with_failure(user_error)
            .with_resume_context(resume_context));
    }

    let requires_content_evidence = route_requires_content_evidence(agent_run_context);
    discard_meta_respond_placeholder_for_content_evidence(
        state,
        task,
        &mut loop_state,
        requires_content_evidence,
    )
    .await;
    let mut finalizer_summary: Option<crate::task_journal::TaskJournalFinalizerSummary> = None;
    let should_try_observed_scalar_fallback = crate::finalizer::should_attempt_observed_fallback(
        loop_state.has_tool_or_skill_output,
        loop_state.has_recoverable_failure_context,
    ) && loop_state.delivery_messages.is_empty();
    if should_try_observed_scalar_fallback {
        if let Some((answer, summary)) =
            direct_scalar_observed_answer(&loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_scalar task_id={}",
                task.task_id
            );
        }
    }

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            super::observed_output::synthesize_answer_from_observed_output(
                state,
                task,
                user_text,
                &loop_state,
                agent_run_context,
            )
            .await
        {
            if matches!(
                summary.disposition,
                Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
            ) && !answer.trim().is_empty()
            {
                finalizer_summary = Some(summary);
                loop_state.last_user_visible_respond = Some(answer.clone());
                append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
                info!(
                    "delivery fallback_from_observed_answer task_id={}",
                    task.task_id
                );
            } else if finalizer_summary.is_none() {
                finalizer_summary = Some(summary);
            }
        }
    }

    let has_authoritative_delivery = !loop_state.delivery_messages.is_empty();
    if finalizer_requires_clarify(
        finalizer_summary.as_ref(),
        requires_content_evidence,
        has_authoritative_delivery,
    ) {
        let clarify_reason = build_finalizer_clarify_reason(finalizer_summary.as_ref());
        let clarify = crate::intent_router::generate_clarify_question(
            state,
            task,
            user_text,
            &clarify_reason,
            None,
        )
        .await;
        let delivery_messages = vec![clarify.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&clarify, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &clarify,
            crate::task_journal::TaskJournalFinalStatus::Clarify,
        );
        return Ok(AskReply::non_llm(clarify.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }

    let (delivery_deduped, _, used_last_respond) =
        crate::finalizer::build_final_delivery_with_priority(
            &loop_state.delivery_messages,
            loop_state.last_user_visible_respond.as_ref(),
        );

    if delivery_deduped.is_empty() {
        let clarify_reason = build_missing_delivery_clarify_reason(finalizer_summary.as_ref());
        let clarify = crate::intent_router::generate_clarify_question(
            state,
            task,
            user_text,
            &clarify_reason,
            None,
        )
        .await;
        let delivery_messages = vec![clarify.clone()];
        let delivery_consistent =
            crate::task_journal::delivery_payload_consistent(&clarify, &delivery_messages);
        let journal = build_loop_journal(
            task,
            user_text,
            &loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_consistent,
            &clarify,
            crate::task_journal::TaskJournalFinalStatus::Clarify,
        );
        return Ok(AskReply::non_llm(clarify.clone())
            .with_messages(delivery_messages)
            .with_task_journal(journal));
    }

    let final_text = delivery_deduped.last().cloned().unwrap_or_default();

    if used_last_respond {
        info!(
            "final_result_source=last_respond task_id={} len={}",
            task.task_id,
            delivery_deduped.len()
        );
    } else if !delivery_deduped.is_empty() {
        info!(
            "final_result_source=delivery_messages task_id={} len={}",
            task.task_id,
            delivery_deduped.len()
        );
    }
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&final_text, &delivery_deduped);

    crate::append_act_plan_log(
        state,
        task,
        "loop_done",
        loop_state.total_steps_executed,
        loop_state.subtask_results.len(),
        loop_state.tool_calls_total,
        &format!(
            "rounds={} messages={} no_progress_count={}",
            loop_state.round_no,
            loop_state.delivery_messages.len(),
            loop_state.consecutive_no_progress
        ),
    );
    let journal = build_loop_journal(
        task,
        user_text,
        &loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_consistent,
        &final_text,
        crate::task_journal::TaskJournalFinalStatus::Success,
    );
    Ok(AskReply::non_llm(final_text)
        .with_messages(delivery_deduped)
        .with_task_journal(journal))
}

#[cfg(test)]
mod tests {
    use super::{
        direct_scalar_observed_answer, finalizer_requires_clarify,
        verify_summary_requires_resume_confirmation,
    };
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    use crate::{
        IntentOutputContract, OutputLocatorKind, OutputResponseShape, ResumeBehavior, RiskCeiling,
        RouteResult, RoutedMode, ScheduleKind,
    };

    fn verify_summary(
        mode: crate::verifier::VerifyMode,
    ) -> crate::task_journal::TaskJournalVerifySummary {
        crate::task_journal::TaskJournalVerifySummary {
            mode,
            approved: true,
            needs_confirmation: true,
            ..Default::default()
        }
    }

    fn finalizer_summary(
        disposition: crate::finalizer::FinalizerDisposition,
    ) -> crate::task_journal::TaskJournalFinalizerSummary {
        crate::task_journal::TaskJournalFinalizerSummary {
            disposition: Some(disposition),
            ..Default::default()
        }
    }

    fn scalar_route_result() -> RouteResult {
        RouteResult {
            routed_mode: RoutedMode::Act,
            resolved_intent: "extract scalar".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            wants_file_delivery: false,
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Filename,
                delivery_intent: Default::default(),
                locator_hint: "package.json".to_string(),
            },
        }
    }

    #[test]
    fn confirmation_resume_requires_enforce_mode() {
        let mut verify = verify_summary(crate::verifier::VerifyMode::ObserveOnly);
        assert!(!verify_summary_requires_resume_confirmation(&verify));

        verify.mode = crate::verifier::VerifyMode::Enforce;
        assert!(verify_summary_requires_resume_confirmation(&verify));

        verify.approved = false;
        assert!(!verify_summary_requires_resume_confirmation(&verify));
    }

    #[test]
    fn content_evidence_routes_require_clarify_without_qualified_completion() {
        assert!(finalizer_requires_clarify(None, true, false));
        assert!(!finalizer_requires_clarify(None, true, true));

        let allow_fallback =
            finalizer_summary(crate::finalizer::FinalizerDisposition::AllowFallback);
        assert!(finalizer_requires_clarify(
            Some(&allow_fallback),
            true,
            false
        ));
        assert!(!finalizer_requires_clarify(
            Some(&allow_fallback),
            true,
            true
        ));

        let qualified =
            finalizer_summary(crate::finalizer::FinalizerDisposition::QualifiedCompletion);
        assert!(!finalizer_requires_clarify(Some(&qualified), true, false));
        assert!(!finalizer_requires_clarify(None, false, false));
    }

    #[test]
    fn direct_scalar_finalize_uses_structured_extract_field_missing_result() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(scalar_route_result()),
            ..Default::default()
        };
        let (answer, summary) =
            direct_scalar_observed_answer(&loop_state, Some(&agent_run_context))
                .expect("scalar fallback should succeed");
        assert_eq!(answer, "name 字段不存在");
        assert_eq!(
            summary.disposition,
            Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
        );
    }
}

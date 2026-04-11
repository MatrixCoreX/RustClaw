use std::path::Path;

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

fn preferred_route_clarify_question(agent_run_context: Option<&AgentRunContext>) -> Option<&str> {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .filter(|route| route.needs_clarify)
        .map(|route| route.clarify_question.trim())
        .filter(|question| !question.is_empty())
}

fn route_requires_file_token(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| {
            route.output_contract.delivery_required
                || matches!(
                    route.output_contract.response_shape,
                    crate::OutputResponseShape::FileToken
                )
        })
        .unwrap_or(false)
}

fn resolve_file_token_from_auto_locator_answer(
    answer: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let trimmed = answer.trim();
    if trimmed.is_empty()
        || trimmed.contains('\n')
        || crate::finalizer::parse_delivery_file_token(trimmed).is_some()
    {
        return None;
    }
    let auto_locator_path = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let auto_path = Path::new(auto_locator_path);

    let resolved = if auto_path.is_file() {
        let file_name = auto_path.file_name().and_then(|v| v.to_str())?;
        if trimmed != file_name {
            return None;
        }
        auto_path
            .canonicalize()
            .unwrap_or_else(|_| auto_path.to_path_buf())
    } else if auto_path.is_dir() {
        let candidate = auto_path.join(trimmed);
        if !candidate.is_file() {
            return None;
        }
        candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf())
    } else {
        return None;
    };

    Some(format!("FILE:{}", resolved.display()))
}

fn normalize_file_token_delivery_from_auto_locator(
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    if !route_requires_file_token(agent_run_context) {
        return;
    }
    let auto_locator_path = agent_run_context.and_then(|ctx| ctx.auto_locator_path.as_deref());

    if let Some(token) = loop_state
        .last_user_visible_respond
        .as_deref()
        .and_then(|answer| resolve_file_token_from_auto_locator_answer(answer, auto_locator_path))
    {
        loop_state.last_user_visible_respond = Some(token);
    }

    for message in &mut loop_state.delivery_messages {
        if let Some(token) = resolve_file_token_from_auto_locator_answer(message, auto_locator_path)
        {
            *message = token;
        }
    }
}

fn enforce_delivery_output_contract(
    state: &AppState,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return;
    };
    if loop_state.delivery_messages.is_empty()
        && loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        return;
    }
    let seed_text = loop_state
        .last_user_visible_respond
        .clone()
        .or_else(|| loop_state.delivery_messages.last().cloned())
        .unwrap_or_default();
    let (normalized_text, normalized_messages) = crate::intercept_response_payload_for_delivery(
        state,
        user_text,
        route.wants_file_delivery,
        &route.output_contract,
        seed_text,
        loop_state.delivery_messages.clone(),
    );
    loop_state.last_user_visible_respond =
        (!normalized_text.trim().is_empty()).then_some(normalized_text);
    loop_state.delivery_messages = normalized_messages;
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
    let answer = if route_prefers_existence_with_path_answer(route) {
        super::observed_output::extract_direct_answer_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .or_else(|| {
            super::observed_output::extract_direct_scalar_from_generic_output(
                loop_state,
                agent_run_context,
            )
        })?
    } else {
        super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )?
    };
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

fn route_prefers_existence_with_path_answer(route: &crate::RouteResult) -> bool {
    if route.output_contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    let intent = route.resolved_intent.trim();
    if intent.is_empty() {
        return false;
    }
    let zh_exists =
        intent.contains("有或没有") || (intent.contains("是否存在") && intent.contains("路径"));
    let en_exists = intent.to_ascii_lowercase().contains("yes or no")
        && intent.to_ascii_lowercase().contains("path");
    zh_exists || en_exists
}

fn direct_structured_observed_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let route = agent_run_context.and_then(|ctx| ctx.route_result.as_ref())?;
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken | crate::OutputResponseShape::Scalar
    ) {
        return None;
    }
    let answer = super::observed_output::extract_direct_answer_from_generic_output(
        loop_state,
        agent_run_context,
    )?;
    if answer.trim().is_empty() {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

async fn direct_publishable_observed_answer(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return None;
    };
    if route.output_contract.requires_content_evidence
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let observed = super::observed_output::extract_latest_generic_successful_output(loop_state)?;
    let answer = observed.body.trim().to_string();
    if answer.is_empty()
        || crate::finalizer::looks_like_planner_artifact(&answer)
        || crate::finalizer::looks_like_internal_trace_artifact(&answer)
        || looks_like_structured_machine_output(&answer)
    {
        return None;
    }
    if looks_like_raw_command_snapshot(&answer)
        && !(observed.skill == "run_cmd" && route_explicitly_requests_command_result(route))
    {
        return None;
    }
    if !crate::semantic_judge::is_publishable_raw(state, task, &answer).await {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion),
            contract_ok: true,
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

fn looks_like_structured_machine_output(answer: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(answer)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
}

fn looks_like_raw_command_snapshot(answer: &str) -> bool {
    let trimmed = answer.trim();
    trimmed.starts_with("exit=")
        && trimmed.contains('\n')
        && (trimmed.contains("\nCOMMAND ")
            || trimmed.contains("(LISTEN)")
            || trimmed.contains("%CPU")
            || trimmed.contains("PID PPID"))
}

fn route_explicitly_requests_command_result(route: &crate::RouteResult) -> bool {
    let intent = route.resolved_intent.trim().to_ascii_lowercase();
    if intent.is_empty() {
        return false;
    }
    (intent.contains("命令") && (intent.contains("执行") || intent.contains("运行")))
        || intent.contains("直接回复执行结果")
        || intent.contains("直接回执行结果")
        || intent.contains("run command")
        || intent.contains("execute command")
        || intent.contains("show command output")
        || intent.contains("raw output")
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
            direct_structured_observed_answer(&loop_state, agent_run_context)
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_structured task_id={}",
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

    if loop_state.delivery_messages.is_empty() {
        if let Some((answer, summary)) =
            direct_publishable_observed_answer(state, task, &loop_state, agent_run_context).await
        {
            finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            info!(
                "delivery fallback_from_observed_raw task_id={}",
                task.task_id
            );
        }
    }

    normalize_file_token_delivery_from_auto_locator(&mut loop_state, agent_run_context);
    enforce_delivery_output_contract(state, user_text, &mut loop_state, agent_run_context);

    let has_authoritative_delivery = !loop_state.delivery_messages.is_empty();
    if finalizer_requires_clarify(
        finalizer_summary.as_ref(),
        requires_content_evidence,
        has_authoritative_delivery,
    ) {
        let clarify_reason = build_finalizer_clarify_reason(finalizer_summary.as_ref());
        let clarify = crate::intent_router::generate_or_reuse_clarify_question(
            state,
            task,
            user_text,
            &clarify_reason,
            None,
            preferred_route_clarify_question(agent_run_context),
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
        let clarify = crate::intent_router::generate_or_reuse_clarify_question(
            state,
            task,
            user_text,
            &clarify_reason,
            None,
            preferred_route_clarify_question(agent_run_context),
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
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        direct_scalar_observed_answer, finalizer_requires_clarify, looks_like_raw_command_snapshot,
        looks_like_structured_machine_output, normalize_file_token_delivery_from_auto_locator,
        resolve_file_token_from_auto_locator_answer, verify_summary_requires_resume_confirmation,
    };
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    use crate::{
        IntentOutputContract, OutputLocatorKind, OutputResponseShape, ResumeBehavior, RiskCeiling,
        RouteResult, RoutedMode, ScheduleKind,
    };

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_loop_finalize_{prefix}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

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
            clarify_question: String::new(),
            schedule_intent: None,
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
    fn preferred_route_clarify_question_only_uses_explicit_route_clarify() {
        let mut route = scalar_route_result();
        route.needs_clarify = true;
        route.clarify_question = "请确认要读取哪个文件？".to_string();
        let ctx = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(
            super::preferred_route_clarify_question(Some(&ctx)),
            Some("请确认要读取哪个文件？")
        );

        let mut route = scalar_route_result();
        route.clarify_question = "不会被复用".to_string();
        let ctx = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        assert_eq!(super::preferred_route_clarify_question(Some(&ctx)), None);
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

    #[test]
    fn direct_scalar_finalize_prefers_presence_plus_path_for_fs_search_presence_queries() {
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "fs_search".to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(
                r#"{"action":"find_name","count":1,"results":["rustclaw.service"],"root":""}"#
                    .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = scalar_route_result();
        route.resolved_intent =
            "检查仓库工作区中是否存在 rustclaw.service 文件，如果存在则返回路径，如果不存在则返回不存在。回答格式只输出有或没有以及路径。"
                .to_string();
        route.output_contract.requires_content_evidence = false;
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            ..Default::default()
        };
        let (answer, summary) =
            direct_scalar_observed_answer(&loop_state, Some(&agent_run_context))
                .expect("presence+path fallback should succeed");
        assert_eq!(answer, "有，路径：rustclaw.service");
        assert_eq!(
            summary.disposition,
            Some(crate::finalizer::FinalizerDisposition::QualifiedCompletion)
        );
    }

    #[test]
    fn raw_publishable_guard_rejects_structured_json_payloads() {
        assert!(looks_like_structured_machine_output(
            r#"{"hostname":"rustclaw-test-host.local","cwd":"/tmp/rustclaw-workspace"}"#
        ));
        assert!(looks_like_structured_machine_output(
            r#"[{"name":"README.md"},{"name":"Cargo.toml"}]"#
        ));
        assert!(!looks_like_structured_machine_output(
            "rustclaw-test-host.local"
        ));
        assert!(!looks_like_structured_machine_output(
            "package_manager=brew"
        ));
    }

    #[test]
    fn raw_publishable_guard_rejects_multi_line_command_snapshots() {
        assert!(looks_like_raw_command_snapshot(
            "exit=0\nCOMMAND PID USER\nclawd 4498 testuser TCP *:8787 (LISTEN)\n"
        ));
        assert!(!looks_like_raw_command_snapshot("testuser"));
    }

    #[test]
    fn file_token_auto_locator_wraps_bare_filename_under_directory() {
        let temp = TempDirGuard::new("file_token_dir");
        let file_path = temp.path().join("report.txt");
        fs::write(&file_path, "hello").expect("write");
        let expected = format!(
            "FILE:{}",
            file_path
                .canonicalize()
                .unwrap_or(file_path.clone())
                .display()
        );
        assert_eq!(
            resolve_file_token_from_auto_locator_answer(
                "report.txt",
                Some(temp.path().to_string_lossy().as_ref())
            )
            .as_deref(),
            Some(expected.as_str())
        );
    }

    #[test]
    fn file_token_auto_locator_normalizes_delivery_messages() {
        let temp = TempDirGuard::new("file_token_messages");
        let file_path = temp.path().join("report.txt");
        fs::write(&file_path, "hello").expect("write");
        let expected = format!(
            "FILE:{}",
            file_path
                .canonicalize()
                .unwrap_or(file_path.clone())
                .display()
        );
        let mut loop_state = super::super::LoopState::new(2);
        loop_state.last_user_visible_respond = Some("report.txt".to_string());
        loop_state.delivery_messages.push("report.txt".to_string());

        let mut route = scalar_route_result();
        route.output_contract.response_shape = OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        let agent_run_context = super::super::AgentRunContext {
            route_result: Some(route),
            auto_locator_path: Some(temp.path().to_string_lossy().to_string()),
            ..Default::default()
        };

        normalize_file_token_delivery_from_auto_locator(&mut loop_state, Some(&agent_run_context));

        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(loop_state.delivery_messages, vec![expected]);
    }
}

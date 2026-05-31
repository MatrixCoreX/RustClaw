use tracing::{info, warn};

use super::{
    append_progress_hint, attempt_ledger, encode_progress_i18n, ensure_task_running,
    execute_actions_once, load_agent_loop_guard_policy, prepare_round_actions, push_round_trace,
    AgentLoopGuardPolicy, AgentRunContext, LoopState, RoundOutcome,
};
use crate::{AgentAction, AppState, AskReply, ClaimedTask, RouteResult};

fn has_authoritative_delivery(loop_state: &LoopState) -> bool {
    !loop_state.delivery_messages.is_empty()
        || loop_state
            .last_user_visible_respond
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
        || loop_state
            .last_publishable_synthesis_output
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
}

fn reply_final_status_is_clarify(reply: &AskReply) -> bool {
    reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.final_status)
        .is_some_and(|status| {
            matches!(status, crate::task_journal::TaskJournalFinalStatus::Clarify)
        })
}

fn route_expects_terminal_user_answer(route_result: &RouteResult) -> bool {
    if route_result.output_contract.delivery_required {
        return false;
    }
    !matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    )
}

fn has_discussion_followup_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| match action {
        AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. } => true,
        AgentAction::Think { .. } => false,
        AgentAction::CallSkill { .. }
        | AgentAction::CallTool { .. }
        | AgentAction::CallCapability { .. } => false,
    })
}

fn has_executable_observation_or_action(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. }
                | AgentAction::CallTool { .. }
                | AgentAction::SynthesizeAnswer { .. }
        )
    })
}

fn last_executable_action(actions: &[AgentAction]) -> Option<&AgentAction> {
    actions.iter().rev().find(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    })
}

fn action_reads_text_content(action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        AgentAction::CallCapability { .. } => return false,
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => return false,
    };
    let normalized_skill = skill.trim().replace('-', "_").to_ascii_lowercase();
    if matches!(normalized_skill.as_str(), "read_file" | "doc_parse") {
        return true;
    }
    normalized_skill == "system_basic"
        && args
            .get("action")
            .and_then(|value| value.as_str())
            .map(|action| action.trim().eq_ignore_ascii_case("read_range"))
            .unwrap_or(false)
}

fn route_needs_workspace_text_evidence_before_observed_finalize(route: &RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route.output_contract.response_shape == crate::OutputResponseShape::Free
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
        && route.output_contract.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.output_contract.locator_hint.trim().is_empty()
}

fn latest_path_batch_facts_all_missing(loop_state: &LoopState) -> bool {
    for step in loop_state.executed_step_results.iter().rev() {
        if !step.is_ok() || step.skill != "system_basic" {
            continue;
        }
        let Some(output) = step.output.as_deref() else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
            continue;
        };
        if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts") {
            continue;
        }
        let Some(facts) = value.get("facts").and_then(|value| value.as_array()) else {
            return false;
        };
        if facts.is_empty() {
            return false;
        }
        return facts
            .iter()
            .all(|fact| fact.get("exists").and_then(|value| value.as_bool()) == Some(false));
    }
    false
}

pub(crate) fn requested_success_marker(
    _agent_run_context: Option<&AgentRunContext>,
) -> Option<&'static str> {
    None
}

fn observed_answer_contains_required_success_marker(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    marker: &str,
) -> bool {
    super::observed_output::extract_direct_answer_from_generic_output(loop_state, agent_run_context)
        .is_some_and(|answer| answer.contains(marker))
        || super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some_and(|answer| answer.contains(marker))
}

fn should_stop_for_observed_finalize(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    if loop_state.execution_recipe.is_active()
        && !matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Done
        )
    {
        return false;
    }
    if loop_state.execution_recipe.needs_validation() {
        return false;
    }
    let route_result = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    let Some(route_result) = route_result else {
        return false;
    };
    if route_result.needs_clarify
        || !loop_state.has_tool_or_skill_output
        || has_authoritative_delivery(loop_state)
    {
        return false;
    }
    if route_needs_workspace_text_evidence_before_observed_finalize(route_result)
        && !has_discussion_followup_action(actions)
        && !last_executable_action(actions).is_some_and(action_reads_text_content)
    {
        return false;
    }
    let required_success_marker = requested_success_marker(agent_run_context);
    let has_direct_observed_answer =
        super::observed_output::extract_direct_answer_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some();
    if route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ExistenceWithPath
        && has_direct_observed_answer
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
        && loop_state.round_no < loop_state.max_rounds
        && latest_path_batch_facts_all_missing(loop_state)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if has_direct_observed_answer
        && route_result.output_contract.response_shape != crate::OutputResponseShape::Scalar
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar {
        if super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some()
        {
            return required_success_marker.is_none_or(|marker| {
                observed_answer_contains_required_success_marker(
                    agent_run_context,
                    loop_state,
                    marker,
                )
            });
        }
        if super::observed_output::scalar_route_prefers_structured_observed_answer(
            route_result,
            loop_state,
        ) && super::observed_output::extract_direct_answer_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some()
        {
            return required_success_marker.is_none_or(|marker| {
                observed_answer_contains_required_success_marker(
                    agent_run_context,
                    loop_state,
                    marker,
                )
            });
        }
    }
    let can_stop = has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && route_expects_terminal_user_answer(route_result)
        && super::observed_output::has_observed_answer_candidates(loop_state);
    can_stop
        && required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        })
}

fn evaluate_round_outcome(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    outcome: &RoundOutcome,
) -> bool {
    if outcome.had_error {
        info!(
            "loop_round_stop task_id={} round={} reason=had_error",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if let Some(reason) = &outcome.stop_signal {
        if reason == "recoverable_failure_continue_round" {
            if !policy.multi_round_enabled {
                info!(
                    "loop_round_stop task_id={} round={} reason=recoverable_failure_multi_round_disabled",
                    task.task_id, loop_state.round_no
                );
                return true;
            }
            if loop_state.round_no >= loop_state.max_rounds {
                if loop_state.recoverable_failure_extra_rounds_used
                    >= policy.recoverable_failure_extra_rounds
                {
                    info!(
                        "loop_round_stop task_id={} round={} reason=recoverable_failure_extra_rounds_exhausted used={} limit={}",
                        task.task_id,
                        loop_state.round_no,
                        loop_state.recoverable_failure_extra_rounds_used,
                        policy.recoverable_failure_extra_rounds
                    );
                    return true;
                }
                loop_state.recoverable_failure_extra_rounds_used += 1;
                loop_state.max_rounds += 1;
                info!(
                    "loop_round_extend task_id={} round={} reason={} new_max_rounds={} used_extra={}",
                    task.task_id,
                    loop_state.round_no,
                    reason,
                    loop_state.max_rounds,
                    loop_state.recoverable_failure_extra_rounds_used
                );
            }
            loop_state.consecutive_no_progress = 0;
            info!(
                "loop_round_continue task_id={} round={} reason={}",
                task.task_id, loop_state.round_no, reason
            );
            return false;
        }
        info!(
            "loop_round_stop task_id={} round={} reason={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            reason,
            crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
        );
        return true;
    }
    if outcome.executed_actions == 0 {
        info!(
            "loop_round_stop task_id={} round={} reason=no_actions",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if outcome.no_progress {
        loop_state.consecutive_no_progress += 1;
    } else {
        loop_state.consecutive_no_progress = 0;
    }
    if loop_state.consecutive_no_progress > policy.no_progress_limit {
        info!(
            "loop_round_stop task_id={} round={} reason=no_progress limit={} count={}",
            task.task_id,
            loop_state.round_no,
            policy.no_progress_limit,
            loop_state.consecutive_no_progress
        );
        return true;
    }
    if !policy.multi_round_enabled {
        info!(
            "loop_round_stop task_id={} round={} reason=multi_round_disabled",
            task.task_id, loop_state.round_no
        );
        return true;
    }
    if loop_state.round_no >= loop_state.max_rounds {
        info!(
            "loop_round_stop task_id={} round={} reason=max_rounds reached={}",
            task.task_id, loop_state.round_no, loop_state.max_rounds
        );
        return true;
    }
    false
}

async fn run_agent_round(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<RoundOutcome, String> {
    info!(
        "loop_round_start task_id={} round={} max_rounds={} total_steps={} tool_calls_total={}",
        task.task_id,
        loop_state.round_no,
        loop_state.max_rounds,
        loop_state.total_steps_executed,
        loop_state.tool_calls_total
    );
    let prepared_round = prepare_round_actions(
        state,
        task,
        goal,
        user_text,
        policy,
        loop_state,
        agent_run_context,
    )
    .await?;
    push_round_trace(loop_state, goal, &prepared_round);
    let actions = prepared_round.actions;
    let mut outcome = execute_actions_once(
        state,
        task,
        goal,
        user_text,
        &actions,
        loop_state,
        policy,
        agent_run_context,
    )
    .await?;
    if outcome.stop_signal.is_none()
        && should_stop_for_observed_finalize(agent_run_context, loop_state, &actions)
    {
        outcome.stop_signal = Some("observed_output_ready".to_string());
    }
    info!(
        "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
        task.task_id,
        loop_state.round_no,
        outcome.executed_actions,
        outcome.no_progress,
        outcome.stop_signal.as_deref().unwrap_or(""),
        crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
    );
    Ok(outcome)
}

fn initial_execution_recipe_spec(
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> crate::execution_recipe::ExecutionRecipeSpec {
    if let Some(spec) = agent_run_context.and_then(|ctx| ctx.execution_recipe_hint) {
        return spec;
    }
    let _ = (goal, user_text);
    warn!(
        "execution_recipe_no_hint_bypass_local_detector route_available={} user_request_available={}",
        agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .is_some(),
        agent_run_context
            .and_then(|ctx| ctx.user_request.as_deref())
            .is_some_and(|text| !text.trim().is_empty())
    );
    crate::execution_recipe::ExecutionRecipeSpec::default()
}

pub(super) async fn run_agent_with_loop(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<AskReply, String> {
    let base_policy = load_agent_loop_guard_policy(state);
    let mut loop_state = LoopState::new(base_policy.max_rounds.max(1));
    super::seed_loop_state_from_agent_context(&mut loop_state, agent_run_context);
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        initial_execution_recipe_spec(goal, user_text, agent_run_context),
    );
    let policy = base_policy.adjusted_for_recipe(loop_state.execution_recipe);
    loop_state.max_rounds = policy.max_rounds.max(1);
    base_policy.apply_recipe_runtime_overrides(&mut loop_state.execution_recipe);
    let mut round = 1usize;
    let mut answer_verifier_retry_count = 0usize;
    loop {
        while round <= loop_state.max_rounds {
            ensure_task_running(state, task)?;
            loop_state.round_no = round;
            super::maybe_publish_execution_recipe_phase_hint(state, task, &mut loop_state);
            let outcome = run_agent_round(
                state,
                task,
                goal,
                user_text,
                &policy,
                &mut loop_state,
                agent_run_context,
            )
            .await?;
            loop_state.last_stop_signal = outcome.stop_signal.clone();
            if evaluate_round_outcome(task, &mut loop_state, &policy, &outcome) {
                break;
            }
            round += 1;
        }
        let pre_finalize_loop_state = loop_state.clone();
        let mut reply = crate::finalize::finalize_loop_reply(
            state,
            task,
            user_text,
            loop_state,
            agent_run_context,
        )
        .await?;
        attach_answer_verifier_if_missing(state, task, user_text, agent_run_context, &mut reply)
            .await;
        let route_result = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
        suppress_answer_verifier_retry_if_structurally_satisfied(&mut reply, route_result);
        if let Some(verifier) = answer_verifier_retry_summary(&reply, route_result).cloned() {
            if answer_verifier_retry_count < policy.answer_verifier_retry_limit
                && policy.multi_round_enabled
            {
                loop_state = pre_finalize_loop_state;
                answer_verifier_retry_count += 1;
                loop_state.has_recoverable_failure_context = true;
                loop_state.delivery_messages.clear();
                loop_state.last_user_visible_respond = None;
                loop_state.last_publishable_synthesis_output = None;
                loop_state.last_stop_signal = Some("answer_verifier_retry".to_string());
                attempt_ledger::record_attempt_with_retry_instruction(
                    &mut loop_state,
                    "answer_verifier",
                    &format!(
                        "missing_evidence_fields={}",
                        verifier.missing_evidence_fields.join(",")
                    ),
                    crate::executor::StepExecutionStatus::Error,
                    &reply.text,
                    Some("answer_incomplete"),
                    &verifier.answer_incomplete_reason,
                    Some(&verifier.retry_instruction),
                );
                append_progress_hint(
                    state,
                    task,
                    &mut loop_state.progress_messages,
                    encode_progress_i18n("telegram.progress.answer_incomplete_retry", &[]),
                );
                if loop_state.round_no >= loop_state.max_rounds {
                    loop_state.max_rounds += 1;
                }
                round = loop_state.round_no + 1;
                info!(
                    "loop_round_extend task_id={} round={} reason=answer_verifier_retry new_max_rounds={}",
                    task.task_id, loop_state.round_no, loop_state.max_rounds
                );
                continue;
            }
            warn!(
                "answer_verifier_retry_exhausted task_id={} retry_count={} limit={} reason={}",
                task.task_id,
                answer_verifier_retry_count,
                policy.answer_verifier_retry_limit,
                crate::truncate_for_log(&verifier.answer_incomplete_reason)
            );
            if try_recover_log_analyze_answer_verifier_gap(user_text, &mut reply) {
                return Ok(reply);
            }
            if try_recover_structured_count_answer_verifier_gap(route_result, user_text, &mut reply)
            {
                return Ok(reply);
            }
            if try_recover_structured_search_answer_verifier_gap(user_text, &mut reply) {
                return Ok(reply);
            }
            if try_recover_generic_path_content_read_range_answer_verifier_gap(
                route_result,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_content_excerpt_summary_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            mark_reply_failed_after_answer_verifier_exhausted(user_text, &mut reply, &verifier);
        }
        return Ok(reply);
    }
}

async fn attach_answer_verifier_if_missing(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    reply: &mut AskReply,
) {
    if reply.should_fail_task || reply_final_status_is_clarify(reply) {
        return;
    }
    let Some(route_result) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return;
    };
    let Some(journal) = reply.task_journal.as_mut() else {
        return;
    };
    if journal.answer_verifier_summary.is_some() {
        return;
    }
    if let Some(answer_verifier) = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        user_text,
        route_result,
        journal,
        &reply.text,
    )
    .await
    {
        journal.record_answer_verifier_summary(answer_verifier);
    }
}

fn answer_verifier_retry_summary<'a>(
    reply: &'a AskReply,
    route_result: Option<&RouteResult>,
) -> Option<&'a crate::task_journal::TaskJournalAnswerVerifierSummary> {
    if reply.should_fail_task || reply_final_status_is_clarify(reply) {
        return None;
    }
    let summary = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())?;
    if answer_verifier_gap_is_structurally_satisfied(reply, route_result) {
        return None;
    }
    summary.high_confidence_retry_gap().then_some(summary)
}

fn suppress_answer_verifier_retry_if_structurally_satisfied(
    reply: &mut AskReply,
    route_result: Option<&RouteResult>,
) -> bool {
    if !answer_verifier_gap_is_structurally_satisfied(reply, route_result) {
        return false;
    }
    let Some(journal) = reply.task_journal.as_mut() else {
        return false;
    };
    let Some(summary) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    info!(
        "answer_verifier_retry_suppressed_structural_satisfaction reason={}",
        crate::truncate_for_log(&summary.answer_incomplete_reason)
    );
    journal.answer_verifier_summary = None;
    true
}

fn answer_verifier_gap_is_structurally_satisfied(
    reply: &AskReply,
    route_result: Option<&RouteResult>,
) -> bool {
    let Some(summary) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !summary.high_confidence_retry_gap() {
        return false;
    }
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::QuantityComparison {
        return quantity_comparison_reply_has_derived_numeric_answer(reply);
    }
    if terminal_content_access_blocker_reply_satisfies_contract(reply, route) {
        return true;
    }
    if let (Some(journal), Some(answer)) = (
        reply.task_journal.as_ref(),
        final_user_answer_candidate(reply),
    ) {
        return crate::answer_verifier::structurally_satisfies_answer_contract(
            route, journal, answer,
        );
    }
    false
}

fn terminal_content_access_blocker_reply_satisfies_contract(
    reply: &AskReply,
    route: &RouteResult,
) -> bool {
    if !route.output_contract.requires_content_evidence {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    let Some(summary) = journal.finalizer_summary.as_ref() else {
        return false;
    };
    if summary.disposition != Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        || summary.completion_ok != Some(true)
        || summary.grounded_ok != Some(true)
    {
        return false;
    }
    journal
        .step_results
        .iter()
        .rev()
        .any(step_has_terminal_content_access_blocker)
}

fn step_has_terminal_content_access_blocker(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    if step.status != crate::executor::StepExecutionStatus::Error {
        return false;
    }
    let Some(error) = step.error_excerpt.as_deref().map(str::trim) else {
        return false;
    };
    if error.is_empty() {
        return false;
    }
    if let Some(policy_block) = crate::skills::parse_policy_block_error(error) {
        return matches!(
            policy_block.reason_code.as_str(),
            "path_outside_workspace" | "path_parent_traversal"
        );
    }
    let Some(structured) = crate::skills::parse_structured_skill_error(error) else {
        return false;
    };
    if structured.error_kind != "permission_denied" {
        return false;
    }
    let effective_skill = if structured.skill.trim().is_empty() {
        step.skill.as_str()
    } else {
        structured.skill.as_str()
    };
    matches!(
        effective_skill.to_ascii_lowercase().as_str(),
        "fs_basic" | "system_basic" | "read_file" | "list_dir"
    )
}

fn final_user_answer_candidate(reply: &AskReply) -> Option<&str> {
    reply
        .messages
        .iter()
        .rev()
        .map(String::as_str)
        .find(|message| {
            let trimmed = message.trim();
            !trimmed.is_empty() && !crate::finalize::is_execution_summary_message(trimmed)
        })
        .or_else(|| {
            let trimmed = reply.text.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })
}

fn collect_size_bytes_from_json(value: &serde_json::Value, out: &mut Vec<u64>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(size) = map
                .get("size_bytes")
                .or_else(|| map.get("total_size_bytes"))
                .and_then(|value| value.as_u64())
            {
                out.push(size);
            }
            for value in map.values() {
                collect_size_bytes_from_json(value, out);
            }
        }
        serde_json::Value::Array(items) => {
            for value in items {
                collect_size_bytes_from_json(value, out);
            }
        }
        _ => {}
    }
}

fn observed_size_bytes(reply: &AskReply) -> Vec<u64> {
    let mut sizes = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return sizes;
    };
    for step in &journal.step_results {
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
            collect_size_bytes_from_json(&value, &mut sizes);
        }
    }
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

fn numeric_literals(text: &str) -> Vec<f64> {
    let mut values = Vec::new();
    let mut token = String::new();
    let mut has_digit = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == ',' || ch == '.' {
            if ch.is_ascii_digit() {
                has_digit = true;
            }
            token.push(ch);
            continue;
        }
        if has_digit {
            push_numeric_literal(&mut values, &token);
        }
        token.clear();
        has_digit = false;
    }
    if has_digit {
        push_numeric_literal(&mut values, &token);
    }
    values
}

fn push_numeric_literal(values: &mut Vec<f64>, token: &str) {
    let normalized = token.trim_matches('.').replace(',', "");
    if normalized.is_empty() || normalized == "." {
        return;
    }
    if let Ok(value) = normalized.parse::<f64>() {
        values.push(value);
    }
}

fn quantity_comparison_reply_has_derived_numeric_answer(reply: &AskReply) -> bool {
    let observed_sizes = observed_size_bytes(reply);
    if observed_sizes.len() < 2 {
        return false;
    }
    let Some(answer) = final_user_answer_candidate(reply) else {
        return false;
    };
    numeric_literals(answer).into_iter().any(|number| {
        !observed_sizes
            .iter()
            .any(|size| (number - *size as f64).abs() < 0.000_001)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LogAnalyzeFinding {
    path: String,
    keyword_counts: Vec<(String, u64)>,
    total_hits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuredSearchFinding {
    action: String,
    count: usize,
    results: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuredCountFinding {
    path: Option<String>,
    total: u64,
    files: Option<u64>,
    dirs: Option<u64>,
    hidden: Option<u64>,
    recursive: Option<bool>,
}

fn try_recover_log_analyze_answer_verifier_gap(user_text: &str, reply: &mut AskReply) -> bool {
    let findings = observed_log_analyze_findings(reply);
    if findings.is_empty() {
        return false;
    }
    let answer = deterministic_log_analyze_summary_text(user_text, &findings);
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!(
        "answer_verifier_retry_exhausted_recovered_with_log_analyze_summary findings={}",
        findings.len()
    );
    true
}

fn try_recover_structured_count_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    user_text: &str,
    reply: &mut AskReply,
) -> bool {
    if !route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ScalarCount
    }) {
        return false;
    }
    let Some(finding) = observed_structured_count_findings(reply).into_iter().next() else {
        return false;
    };
    let answer = deterministic_structured_count_summary_text(user_text, &finding);
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!(
        "answer_verifier_retry_exhausted_recovered_with_structured_count total={}",
        finding.total
    );
    true
}

fn try_recover_structured_search_answer_verifier_gap(
    user_text: &str,
    reply: &mut AskReply,
) -> bool {
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap() {
        return false;
    }
    if !structured_search_verifier_requests_full_candidates(verifier) {
        return false;
    }
    let Some(finding) = observed_structured_search_findings(reply)
        .into_iter()
        .max_by(|left, right| {
            left.count
                .cmp(&right.count)
                .then_with(|| left.results.len().cmp(&right.results.len()))
                .then_with(|| right.action.cmp(&left.action))
        })
    else {
        return false;
    };
    if finding.results.is_empty() || finding.count > finding.results.len() {
        return false;
    }
    let answer = deterministic_structured_search_summary_text(user_text, &finding);
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!(
        "answer_verifier_retry_exhausted_recovered_with_structured_search_results action={} count={}",
        finding.action, finding.count
    );
    true
}

fn try_recover_content_excerpt_summary_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    if !route_result.is_some_and(|route| {
        route
            .output_contract
            .semantic_kind
            .is_content_excerpt_summary()
    }) {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap() {
        return false;
    }
    let Some(answer) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| {
            journal
                .step_results
                .iter()
                .rev()
                .find(|step| {
                    step.skill == "synthesize_answer"
                        && step.status == crate::executor::StepExecutionStatus::Ok
                        && step
                            .output_excerpt
                            .as_deref()
                            .is_some_and(|text| !text.trim().is_empty())
                })
                .and_then(|step| step.output_excerpt.as_deref())
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
    else {
        return false;
    };
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_content_excerpt_summary_synthesis");
    true
}

fn try_recover_generic_path_content_read_range_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::None
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap()
        || !verifier
            .missing_evidence_fields
            .iter()
            .any(|field| matches!(field.as_str(), "path" | "content_excerpt"))
    {
        return false;
    }
    let Some(answer) = reply
        .task_journal
        .as_ref()
        .and_then(latest_read_range_content_evidence_answer)
    else {
        return false;
    };
    let messages = vec![answer.clone()];
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_generic_path_content_read_range");
    true
}

fn latest_read_range_content_evidence_answer(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| {
            matches!(step.skill.as_str(), "fs_basic" | "system_basic")
                && step.status == crate::executor::StepExecutionStatus::Ok
        })
        .find_map(|step| read_range_content_evidence_answer(step.output_excerpt.as_deref()?))
}

fn read_range_content_evidence_answer(output: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    if !matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("read_range" | "read_text_range")
    ) {
        return None;
    }
    let _path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let excerpt = value
        .get("excerpt")
        .and_then(|value| value.as_str())
        .map(sanitize_read_range_content_excerpt)
        .filter(|excerpt| !excerpt.trim().is_empty())?;
    Some(excerpt)
}

fn sanitize_read_range_content_excerpt(excerpt: &str) -> String {
    excerpt
        .lines()
        .map(str::trim_end)
        .map(|line| {
            line.split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start())
                .unwrap_or(line)
        })
        .map(crate::visible_text::sanitize_user_visible_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn structured_search_verifier_requests_full_candidates(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> bool {
    verifier
        .missing_evidence_fields
        .iter()
        .any(|field| matches!(field.as_str(), "candidates" | "results" | "paths" | "files"))
}

fn observed_log_analyze_findings(reply: &AskReply) -> Vec<LogAnalyzeFinding> {
    let mut findings = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return findings;
    };
    for step in &journal.step_results {
        if !step.skill.eq_ignore_ascii_case("log_analyze")
            || step.status != crate::executor::StepExecutionStatus::Ok
        {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(finding) = parse_log_analyze_finding(output) else {
            continue;
        };
        findings.push(finding);
    }
    findings.sort_by(|left, right| {
        right
            .total_hits
            .cmp(&left.total_hits)
            .then_with(|| left.path.cmp(&right.path))
    });
    findings
}

fn observed_structured_count_findings(reply: &AskReply) -> Vec<StructuredCountFinding> {
    let mut findings = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return findings;
    };
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        if !matches!(step.skill.as_str(), "fs_basic" | "system_basic") {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(finding) = parse_structured_count_finding(output) else {
            continue;
        };
        findings.push(finding);
    }
    findings
}

fn observed_structured_search_findings(reply: &AskReply) -> Vec<StructuredSearchFinding> {
    let mut findings = Vec::new();
    let Some(journal) = reply.task_journal.as_ref() else {
        return findings;
    };
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        if !matches!(
            step.skill.as_str(),
            "fs_basic" | "fs_search" | "system_basic"
        ) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        let Some(finding) = parse_structured_search_finding(output) else {
            continue;
        };
        findings.push(finding);
    }
    findings
}

fn parse_structured_count_finding(output: &str) -> Option<StructuredCountFinding> {
    let value =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<serde_json::Value>(output)?;
    let action = value
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    if !matches!(action.as_str(), "count_inventory" | "inventory_dir") {
        return None;
    }
    let counts = value.get("counts")?;
    let total = counts.get("total").and_then(|value| value.as_u64())?;
    Some(StructuredCountFinding {
        path: value
            .get("path")
            .or_else(|| value.get("resolved_path"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
        total,
        files: counts.get("files").and_then(|value| value.as_u64()),
        dirs: counts.get("dirs").and_then(|value| value.as_u64()),
        hidden: counts.get("hidden").and_then(|value| value.as_u64()),
        recursive: value.get("recursive").and_then(|value| value.as_bool()),
    })
}

fn parse_structured_search_finding(output: &str) -> Option<StructuredSearchFinding> {
    let value =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<serde_json::Value>(output)?;
    let action = value
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    if !structured_search_action_has_candidate_list(&action) {
        return None;
    }
    let raw_results = value.get("results").and_then(|value| value.as_array())?;
    let mut results = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for item in raw_results {
        let Some(token) = structured_search_result_token(item) else {
            continue;
        };
        if seen.insert(token.clone()) {
            results.push(token);
        }
    }
    if results.is_empty() {
        return None;
    }
    let count = value
        .get("count")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or(results.len());
    Some(StructuredSearchFinding {
        action,
        count,
        results,
    })
}

fn structured_search_action_has_candidate_list(action: &str) -> bool {
    matches!(
        action,
        "find_name" | "find_ext" | "find_entries" | "find_path" | "search"
    )
}

fn structured_search_result_token(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return non_empty_structured_search_token(text);
    }
    let object = value.as_object()?;
    for key in [
        "path",
        "relative_path",
        "full_path",
        "name",
        "entry",
        "file",
        "filename",
    ] {
        if let Some(text) = object.get(key).and_then(|value| value.as_str()) {
            if let Some(token) = non_empty_structured_search_token(text) {
                return Some(token);
            }
        }
    }
    None
}

fn non_empty_structured_search_token(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_log_analyze_finding(output: &str) -> Option<LogAnalyzeFinding> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        let path = value
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_string();
        let mut keyword_counts = value
            .get("keyword_counts")
            .and_then(|value| value.as_object())
            .map(|counts| {
                counts
                    .iter()
                    .filter_map(|(key, value)| value.as_u64().map(|count| (key.clone(), count)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return build_log_analyze_finding(path, &mut keyword_counts);
    }
    let path = extract_json_string_field(output, "path")?;
    let mut keyword_counts = extract_keyword_counts(output);
    build_log_analyze_finding(path, &mut keyword_counts)
}

fn build_log_analyze_finding(
    path: String,
    keyword_counts: &mut Vec<(String, u64)>,
) -> Option<LogAnalyzeFinding> {
    keyword_counts.retain(|(key, count)| !key.trim().is_empty() && *count > 0);
    if keyword_counts.is_empty() {
        return None;
    }
    keyword_counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let total_hits = keyword_counts.iter().map(|(_, count)| *count).sum::<u64>();
    Some(LogAnalyzeFinding {
        path,
        keyword_counts: keyword_counts.clone(),
        total_hits,
    })
}

fn extract_keyword_counts(output: &str) -> Vec<(String, u64)> {
    let Some(marker_pos) = output.find("\"keyword_counts\"") else {
        return Vec::new();
    };
    let after_marker = &output[marker_pos + "\"keyword_counts\"".len()..];
    let Some(colon_pos) = after_marker.find(':') else {
        return Vec::new();
    };
    let after_colon = &after_marker[colon_pos + 1..];
    let Some(open_rel) = after_colon.find('{') else {
        return Vec::new();
    };
    let object_start = colon_pos + 1 + open_rel;
    let Some(object_end) = find_matching_json_object_end(after_marker, object_start) else {
        return Vec::new();
    };
    let inner = &after_marker[object_start + 1..object_end];
    inner
        .split(',')
        .filter_map(|part| {
            let (key, value) = part.split_once(':')?;
            let key = key.trim().trim_matches('"').to_string();
            let count = value.trim().parse::<u64>().ok()?;
            Some((key, count))
        })
        .collect()
}

fn find_matching_json_object_end(input: &str, open_pos: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    if bytes.get(open_pos).copied() != Some(b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in input.char_indices().skip_while(|(idx, _)| *idx < open_pos) {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_json_string_field(input: &str, field: &str) -> Option<String> {
    let marker = format!("\"{field}\"");
    let mut offset = 0usize;
    while let Some(rel_pos) = input[offset..].find(&marker) {
        let marker_end = offset + rel_pos + marker.len();
        let after_marker = input[marker_end..].trim_start();
        let Some(after_colon) = after_marker.strip_prefix(':') else {
            offset = marker_end;
            continue;
        };
        return parse_json_string(after_colon.trim_start());
    }
    None
}

fn parse_json_string(input: &str) -> Option<String> {
    let mut chars = input.chars();
    if chars.next()? != '"' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            out.push(match ch {
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

fn deterministic_log_analyze_summary_text(
    user_text: &str,
    findings: &[LogAnalyzeFinding],
) -> String {
    let prefer_english = crate::language_policy::request_language_hint(user_text) == "en";
    let mut sorted = findings.to_vec();
    sorted.sort_by(|left, right| {
        right
            .total_hits
            .cmp(&left.total_hits)
            .then_with(|| left.path.cmp(&right.path))
    });
    let top = &sorted[0];
    let overview = sorted
        .iter()
        .take(4)
        .map(|finding| {
            format!(
                "{}: {}",
                display_log_path(&finding.path),
                format_keyword_counts(&finding.keyword_counts)
            )
        })
        .collect::<Vec<_>>()
        .join(if prefer_english { "; " } else { "；" });
    if prefer_english {
        format!(
            "Most notable: `{}` has the heaviest recent signal ({}). Also checked other log files in the directory; summary: {}.",
            display_log_path(&top.path),
            format_keyword_counts(&top.keyword_counts),
            overview
        )
    } else {
        format!(
            "最值得注意的是 `{}`：{}，这是当前已分析日志里异常信号最重的文件；同时也看了 logs 目录里的其他日志，简要汇总：{}。",
            display_log_path(&top.path),
            format_keyword_counts(&top.keyword_counts),
            overview
        )
    }
}

fn deterministic_structured_search_summary_text(
    user_text: &str,
    finding: &StructuredSearchFinding,
) -> String {
    let count = finding.count.max(finding.results.len());
    let prefer_english = crate::language_policy::request_language_hint(user_text) == "en";
    let mut lines = Vec::new();
    if prefer_english {
        lines.push(format!("Found {count} candidates:"));
    } else {
        lines.push(format!("找到 {count} 个候选："));
    }
    for (idx, result) in finding.results.iter().enumerate() {
        lines.push(format!("{}. {}", idx + 1, result));
    }
    lines.join("\n")
}

fn deterministic_structured_count_summary_text(
    user_text: &str,
    finding: &StructuredCountFinding,
) -> String {
    let prefer_english = crate::language_policy::request_language_hint(user_text) == "en";
    let scope = finding.path.as_deref().unwrap_or(if prefer_english {
        "the requested scope"
    } else {
        "目标范围"
    });
    let direct = finding.recursive == Some(false);
    match (
        prefer_english,
        direct,
        finding.files,
        finding.dirs,
        finding.hidden,
    ) {
        (true, true, Some(files), Some(dirs), Some(hidden)) => format!(
            "{scope} has {} direct entries: {files} files, {dirs} directories, {hidden} hidden.",
            finding.total
        ),
        (true, true, Some(files), Some(dirs), None) => format!(
            "{scope} has {} direct entries: {files} files and {dirs} directories.",
            finding.total
        ),
        (true, _, Some(files), Some(dirs), _) => format!(
            "{scope} has {} entries: {files} files and {dirs} directories.",
            finding.total
        ),
        (true, true, _, _, _) => {
            format!("{scope} has {} direct entries.", finding.total)
        }
        (true, _, _, _, _) => format!("{scope} has {} entries.", finding.total),
        (false, true, Some(files), Some(dirs), Some(hidden)) => format!(
            "{scope} 共有 {} 个直接子项：文件 {files} 个，目录 {dirs} 个，隐藏项 {hidden} 个。",
            finding.total
        ),
        (false, true, Some(files), Some(dirs), None) => format!(
            "{scope} 共有 {} 个直接子项：文件 {files} 个，目录 {dirs} 个。",
            finding.total
        ),
        (false, _, Some(files), Some(dirs), _) => format!(
            "{scope} 共有 {} 个子项：文件 {files} 个，目录 {dirs} 个。",
            finding.total
        ),
        (false, true, _, _, _) => format!("{scope} 共有 {} 个直接子项。", finding.total),
        (false, _, _, _, _) => format!("{scope} 共有 {} 个子项。", finding.total),
    }
}

fn display_log_path(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or(path)
}

fn format_keyword_counts(counts: &[(String, u64)]) -> String {
    counts
        .iter()
        .take(5)
        .map(|(key, count)| format!("{key} {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn mark_reply_failed_after_answer_verifier_exhausted(
    user_text: &str,
    reply: &mut AskReply,
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) {
    let message = answer_verifier_exhausted_failure_text(user_text, verifier);
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(message.clone());
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.record_final_answer(&message);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    }
    reply.text = message.clone();
    reply.messages = messages;
    reply.should_fail_task = true;
    reply.error_text = Some(message);
}

fn answer_verifier_exhausted_failure_text(
    user_text: &str,
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> String {
    let reason = verifier.answer_incomplete_reason.trim();
    let reason = if reason.is_empty() {
        "answer verifier reported the final answer is incomplete"
    } else {
        reason
    };
    if crate::language_policy::request_language_hint(user_text) == "en" {
        format!(
            "I could not produce a final answer that satisfies the request after retrying. Verification issue: {reason}"
        )
    } else {
        format!("我重试后仍没能生成满足请求要求的最终回答。校验问题：{reason}")
    }
}

#[cfg(test)]
#[path = "loop_control_tests.rs"]
mod tests;

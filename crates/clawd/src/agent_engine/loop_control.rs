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
            if let Some(size) = map.get("size_bytes").and_then(|value| value.as_u64()) {
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
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(answer.clone());
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
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(answer.clone());
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
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(answer.clone());
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
        route.output_contract.semantic_kind == crate::OutputSemanticKind::ContentExcerptSummary
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
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(answer.clone());
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
mod tests {
    use super::{
        answer_verifier_retry_summary, evaluate_round_outcome, initial_execution_recipe_spec,
        mark_reply_failed_after_answer_verifier_exhausted, parse_log_analyze_finding,
        should_stop_for_observed_finalize,
        suppress_answer_verifier_retry_if_structurally_satisfied,
        try_recover_content_excerpt_summary_answer_verifier_gap,
        try_recover_log_analyze_answer_verifier_gap,
        try_recover_structured_count_answer_verifier_gap,
        try_recover_structured_search_answer_verifier_gap, AgentLoopGuardPolicy, RoundOutcome,
    };
    use crate::{
        agent_engine::{AgentRunContext, LoopState},
        execution_recipe::{
            ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
            ExecutionRecipeSpec, ExecutionRecipeTargetScope,
        },
        executor::{StepExecutionResult, StepExecutionStatus},
        AgentAction, AskReply, ClaimedTask, IntentOutputContract, OutputDeliveryIntent,
        OutputLocatorKind, OutputResponseShape, OutputSemanticKind, ResumeBehavior, RiskCeiling,
        RouteResult, ScheduleKind,
    };
    use serde_json::json;

    fn route_result(shape: OutputResponseShape) -> RouteResult {
        RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "test".to_string(),
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
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: shape,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        }
    }

    fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
        StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        }
    }

    fn test_task() -> ClaimedTask {
        ClaimedTask {
            task_id: "task-loop-control".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "telegram".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    #[test]
    fn answer_verifier_retry_summary_requires_retryable_high_confidence_gap() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: vec!["path".to_string()],
                answer_incomplete_reason: "missing fallback path".to_string(),
                should_retry: true,
                retry_instruction: "search fallback path".to_string(),
                confidence: 0.8,
            });
        let reply = AskReply::non_llm("wrong path".to_string()).with_task_journal(journal);

        let summary = answer_verifier_retry_summary(&reply, None).expect("retry gap");
        assert_eq!(summary.missing_evidence_fields, vec!["path"]);
    }

    #[test]
    fn answer_verifier_retry_summary_uses_high_confidence_gap_even_without_flag() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: Vec::new(),
                answer_incomplete_reason: "candidate contradicts observed evidence".to_string(),
                should_retry: false,
                retry_instruction: String::new(),
                confidence: 0.95,
            });
        let reply = AskReply::non_llm("wrong answer".to_string()).with_task_journal(journal);

        assert!(answer_verifier_retry_summary(&reply, None).is_some());
    }

    #[test]
    fn answer_verifier_retry_summary_skips_clarify_final_status() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: vec!["path".to_string()],
                answer_incomplete_reason: "missing fallback path".to_string(),
                should_retry: true,
                retry_instruction: "search fallback path".to_string(),
                confidence: 0.8,
            });
        let reply =
            AskReply::non_llm("please provide the path".to_string()).with_task_journal(journal);

        assert!(answer_verifier_retry_summary(&reply, None).is_none());
    }

    #[test]
    fn quantity_comparison_structural_answer_suppresses_false_verifier_retry() {
        let mut route = route_result(OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                r#"{"action":"path_batch_facts","facts":[{"exists":true,"fact":{"path":"Cargo.lock","size_bytes":121647}},{"exists":true,"fact":{"path":"Cargo.toml","size_bytes":2606}}]}"#
                    .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: Vec::new(),
                answer_incomplete_reason: "answer only reports the file sizes without ratio"
                    .to_string(),
                should_retry: true,
                retry_instruction: "calculate the ratio".to_string(),
                confidence: 0.95,
            });
        let mut reply = AskReply::non_llm(
            "Cargo.lock 大小为 121,647 字节，Cargo.toml 大小为 2,606 字节。Cargo.lock 大约是 Cargo.toml 的 46.7 倍。"
                .to_string(),
        )
        .with_messages(vec![
            "**执行过程**\n1. 调用工具 `fs_basic`。".to_string(),
            "Cargo.lock 大小为 121,647 字节，Cargo.toml 大小为 2,606 字节。Cargo.lock 大约是 Cargo.toml 的 46.7 倍。"
                .to_string(),
        ])
        .with_task_journal(journal);

        assert!(suppress_answer_verifier_retry_if_structurally_satisfied(
            &mut reply,
            Some(&route)
        ));
        assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());
        assert!(reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.answer_verifier_summary.as_ref())
            .is_none());
    }

    #[test]
    fn file_token_delivery_suppresses_list_count_verifier_retry_when_grounded() {
        let root = std::env::temp_dir().join(format!(
            "rustclaw-loop-control-file-token-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        let file = root.join("report.txt");
        std::fs::write(&file, "report").expect("write temp file");

        let mut route = route_result(OutputResponseShape::FileToken);
        route.wants_file_delivery = true;
        route.output_contract.delivery_required = true;
        route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "inventory_dir",
                        "resolved_path": root.display().to_string(),
                        "names": ["report.txt", "other.txt"],
                        "entries": [
                            {
                                "kind": "file",
                                "name": "report.txt",
                                "path": file.display().to_string()
                            }
                        ]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: Vec::new(),
                answer_incomplete_reason:
                    "answer provides only 1 file path when evidence shows the directory contains many files"
                        .to_string(),
                should_retry: true,
                retry_instruction: "list all files".to_string(),
                confidence: 0.95,
            });
        let mut reply = AskReply::non_llm(format!("FILE:{}", file.display()))
            .with_messages(vec![
                "**执行过程**\n1. 调用工具 `fs_basic`。".to_string(),
                format!("FILE:{}", file.display()),
            ])
            .with_task_journal(journal);

        assert!(suppress_answer_verifier_retry_if_structurally_satisfied(
            &mut reply,
            Some(&route)
        ));
        assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_none());

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn file_token_delivery_does_not_suppress_when_token_is_not_grounded() {
        let root = std::env::temp_dir().join(format!(
            "rustclaw-loop-control-file-token-ungrounded-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        let observed = root.join("observed.txt");
        let ungrounded = root.join("ungrounded.txt");
        std::fs::write(&observed, "observed").expect("write observed file");
        std::fs::write(&ungrounded, "ungrounded").expect("write ungrounded file");

        let mut route = route_result(OutputResponseShape::FileToken);
        route.wants_file_delivery = true;
        route.output_contract.delivery_required = true;
        route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "inventory_dir",
                        "resolved_path": root.display().to_string(),
                        "entries": [
                            {
                                "kind": "file",
                                "name": "observed.txt",
                                "path": observed.display().to_string()
                            }
                        ]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: Vec::new(),
                answer_incomplete_reason: "candidate file is not supported by evidence".to_string(),
                should_retry: true,
                retry_instruction: "select a grounded file".to_string(),
                confidence: 0.95,
            });
        let mut reply =
            AskReply::non_llm(format!("FILE:{}", ungrounded.display())).with_task_journal(journal);

        assert!(!suppress_answer_verifier_retry_if_structurally_satisfied(
            &mut reply,
            Some(&route)
        ));
        assert!(answer_verifier_retry_summary(&reply, Some(&route)).is_some());

        let _ = std::fs::remove_file(&observed);
        let _ = std::fs::remove_file(&ungrounded);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parses_truncated_log_analyze_output_prefix() {
        let finding = parse_log_analyze_finding(
            r#"{"keyword_counts":{"error":115,"failed":48,"panic":23,"timeout":26,"warn":72},"path":"/tmp/logs/clawd.run.log","recent_matches":["... ...(truncated)""#,
        )
        .expect("truncated prefix still contains counts and path");

        assert_eq!(finding.path, "/tmp/logs/clawd.run.log");
        assert_eq!(finding.total_hits, 284);
        assert_eq!(finding.keyword_counts[0], ("error".to_string(), 115));
    }

    #[test]
    fn log_analyze_verifier_exhaustion_recovers_with_structural_summary() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: vec!["clawd.run.log".to_string()],
                answer_incomplete_reason: "candidate omitted clawd.run.log counts".to_string(),
                should_retry: true,
                retry_instruction: "include every analyzed log".to_string(),
                confidence: 0.95,
            });
        journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "log_analyze".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                r#"{"keyword_counts":{"error":1286,"failed":939,"timeout":308},"path":"/logs/model_io.log.2026-05-13","recent_matches":[]}"#
                    .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
        journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_2".to_string(),
            skill: "log_analyze".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                r#"{"keyword_counts":{"error":115,"warn":72,"failed":48},"path":"/logs/clawd.run.log","recent_matches":["...(truncated)"]}"#
                    .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut reply = AskReply::non_llm("partial answer".to_string())
            .with_messages(vec![
                "**执行过程**\n1. 调用技能 `log_analyze`。".to_string(),
                "partial answer".to_string(),
            ])
            .with_task_journal(journal);

        assert!(try_recover_log_analyze_answer_verifier_gap(
            "快速看一下 logs 目录里最近最值得注意的错误或异常",
            &mut reply
        ));

        assert!(!reply.should_fail_task);
        assert!(reply.text.contains("model_io.log.2026-05-13"));
        assert!(reply.text.contains("clawd.run.log"));
        assert!(reply.text.contains("error 115"));
        let journal = reply.task_journal.as_ref().expect("journal");
        assert_eq!(
            journal.final_status,
            Some(crate::task_journal::TaskJournalFinalStatus::Success)
        );
        assert!(journal.answer_verifier_summary.is_none());
    }

    #[test]
    fn structured_search_verifier_exhaustion_recovers_with_full_candidate_list() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: vec!["candidates".to_string()],
                answer_incomplete_reason:
                    "answer reports 1 README file but observed evidence shows 3 README files"
                        .to_string(),
                should_retry: true,
                retry_instruction: "answer from the full observed results array".to_string(),
                confidence: 0.94,
            });
        journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                r#"{"action":"find_name","count":3,"patterns":["README"],"results":["README.md","UI/README.md","docs/README.md"],"root":"/repo"}"#
                    .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut reply = AskReply::non_llm("Found README.md.".to_string())
            .with_messages(vec![
                "**Execution**\n1. Ran tool `fs_basic`.".to_string(),
                "Found README.md.".to_string(),
            ])
            .with_task_journal(journal);

        assert!(try_recover_structured_search_answer_verifier_gap(
            "Find files named README under the current repo.",
            &mut reply
        ));

        assert!(!reply.should_fail_task);
        assert!(reply.text.contains("Found 3 candidates"));
        assert!(reply.text.contains("README.md"));
        assert!(reply.text.contains("UI/README.md"));
        assert!(reply.text.contains("docs/README.md"));
        let journal = reply.task_journal.as_ref().expect("journal");
        assert_eq!(
            journal.final_status,
            Some(crate::task_journal::TaskJournalFinalStatus::Success)
        );
        assert!(journal.answer_verifier_summary.is_none());
    }

    #[test]
    fn structured_count_verifier_exhaustion_recovers_with_count_inventory() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: vec!["count".to_string()],
                answer_incomplete_reason:
                    "answer asks to rerun instead of delivering observed count".to_string(),
                should_retry: true,
                retry_instruction: "use the observed counts.total field".to_string(),
                confidence: 0.94,
            });
        journal.step_results.push(crate::task_journal::TaskJournalStepTrace {
            step_id: "step_1".to_string(),
            skill: "fs_basic".to_string(),
            status: StepExecutionStatus::Ok,
            output_excerpt: Some(
                r#"{"action":"count_inventory","counts":{"dirs":6,"files":58,"hidden":0,"total":64},"path":"/repo/scripts","recursive":false}"#
                    .to_string(),
            ),
            error_excerpt: None,
            started_at: 0,
            finished_at: 0,
        });
        let mut route = route_result(OutputResponseShape::OneSentence);
        route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
        let mut reply = AskReply::non_llm("需要重新触发计数任务。".to_string())
            .with_messages(vec![
                "**执行过程**\n1. 调用工具 `fs_basic`。".to_string(),
                "需要重新触发计数任务。".to_string(),
            ])
            .with_task_journal(journal);

        assert!(try_recover_structured_count_answer_verifier_gap(
            Some(&route),
            "先数一下 scripts 目录直接有多少个子项",
            &mut reply
        ));

        assert!(!reply.should_fail_task);
        assert!(reply.text.contains("64"));
        assert!(reply.text.contains("58"));
        assert!(reply.text.contains("6"));
        let journal = reply.task_journal.as_ref().expect("journal");
        assert_eq!(
            journal.final_status,
            Some(crate::task_journal::TaskJournalFinalStatus::Success)
        );
        assert!(journal.answer_verifier_summary.is_none());
    }

    #[test]
    fn content_excerpt_summary_verifier_exhaustion_recovers_with_synthesis_output() {
        let mut route = route_result(OutputResponseShape::Free);
        route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal.record_route_result(&route);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        journal.answer_verifier_summary =
            Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
                pass: false,
                missing_evidence_fields: vec!["content_excerpt".to_string()],
                answer_incomplete_reason: "final answer dropped synthesized summary".to_string(),
                should_retry: true,
                retry_instruction: "use the full synthesized summary".to_string(),
                confidence: 0.94,
            });
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "synthesize_answer".to_string(),
                status: StepExecutionStatus::Ok,
                output_excerpt: Some(
                    "Summary from observed excerpt covering all required facts.".to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });
        let mut reply = AskReply::non_llm("partial answer".to_string())
            .with_messages(vec![
                "**Execution**\n1. Read file excerpt.".to_string(),
                "partial answer".to_string(),
            ])
            .with_task_journal(journal);

        assert!(try_recover_content_excerpt_summary_answer_verifier_gap(
            Some(&route),
            &mut reply
        ));

        assert!(!reply.should_fail_task);
        assert_eq!(
            reply.text,
            "Summary from observed excerpt covering all required facts."
        );
        let journal = reply.task_journal.as_ref().expect("journal");
        assert_eq!(
            journal.final_status,
            Some(crate::task_journal::TaskJournalFinalStatus::Success)
        );
        assert!(journal.answer_verifier_summary.is_none());
    }

    #[test]
    fn answer_verifier_exhaustion_marks_reply_failure() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        journal.record_final_answer("old answer");
        let verifier = crate::task_journal::TaskJournalAnswerVerifierSummary {
            pass: false,
            missing_evidence_fields: vec!["output_format".to_string()],
            answer_incomplete_reason: "expected exactly five paths".to_string(),
            should_retry: true,
            retry_instruction: "select five paths".to_string(),
            confidence: 0.95,
        };
        journal.answer_verifier_summary = Some(verifier.clone());
        let mut reply = AskReply::non_llm("old answer".to_string())
            .with_messages(vec![
                "**Execution**\n1. Ran tool `fs_basic`.".to_string(),
                "old answer".to_string(),
            ])
            .with_task_journal(journal);

        mark_reply_failed_after_answer_verifier_exhausted("Find five paths", &mut reply, &verifier);

        assert!(reply.should_fail_task);
        assert_eq!(reply.messages.len(), 2);
        assert!(reply.messages[0].starts_with("**Execution**"));
        assert!(reply.text.contains("Verification issue"));
        let journal = reply.task_journal.as_ref().expect("journal");
        assert_eq!(
            journal.final_status,
            Some(crate::task_journal::TaskJournalFinalStatus::Failure)
        );
        assert_eq!(journal.final_answer.as_deref(), Some(reply.text.as_str()));
    }

    fn test_policy() -> AgentLoopGuardPolicy {
        AgentLoopGuardPolicy {
            max_steps: 8,
            max_rounds: 4,
            recoverable_failure_extra_rounds: 1,
            repeat_action_limit: 3,
            no_progress_limit: 1,
            multi_round_enabled: true,
            answer_verifier_retry_limit: 2,
            ops_closed_loop: Default::default(),
        }
    }

    #[test]
    fn observed_scalar_output_can_stop_loop_without_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"extract_field"}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Scalar)),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn observed_config_basic_scalar_output_can_stop_loop_without_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"run_cmd.planner_kind","value_text":"tool","value":"tool","value_type":"string"}"#,
        ));
        let actions = vec![AgentAction::CallTool {
            tool: "config_basic".to_string(),
            args: json!({"action":"read_field","path":"configs/skills_registry.toml","field_path":"run_cmd.planner_kind"}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Strict)),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn observation_only_freeform_round_can_stop_for_observed_fallback() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            "README.md\ndocs/\ncrates/\n",
        ));
        let actions = vec![AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path":"."}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Free)),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn unscoped_workspace_evidence_drafting_does_not_stop_on_search_only() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_search",
            r#"{"action":"find_name","count":2,"results":["README.md","USAGE.md"]}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.resolved_intent =
            "Write a short setup note grounded in the current workspace docs".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![AgentAction::CallSkill {
            skill: "fs_search".to_string(),
            args: json!({"action":"find_name","pattern":"README"}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn unscoped_workspace_evidence_drafting_can_stop_after_doc_read() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"read_range","path":"README.md","excerpt":"1|# RustClaw\n2|## Setup"}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.resolved_intent =
            "Write a short setup note grounded in the current workspace docs".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.locator_hint.clear();
        route.output_contract.semantic_kind = OutputSemanticKind::None;
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"read_range","path":"README.md","mode":"head","n":120}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn hidden_entries_scalar_output_can_stop_before_synthesis_followup() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "list_dir",
            ".git\nREADME.md\n.env\nsrc\n",
        ));
        let mut route = route_result(OutputResponseShape::Scalar);
        route.resolved_intent =
            "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::HiddenEntriesCheck;
        route.output_contract.locator_hint = ".".to_string();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: json!({"path":"."}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn fs_basic_inventory_names_can_stop_before_synthesis_followup() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r#"{"action":"inventory_dir","path":"/tmp/document","resolved_path":"/tmp/document","files_only":true,"names_only":true,"names":["a.txt","b.md","c.png"]}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.ask_mode = crate::AskMode::planner_execute_plain();
        route.resolved_intent = "List file names from a known directory.".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::FileNames;
        route.output_contract.locator_hint = "document".to_string();
        let actions = vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: json!({"action":"list_dir","path":"/tmp/document","files_only":true,"names_only":true}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
        ];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn existence_with_path_free_output_can_stop_before_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.round_no = 1;
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"exists":true,"fact":{"kind":"file","path":"rustclaw.service","resolved_path":"/home/guagua/rustclaw/rustclaw.service","size_bytes":1190},"path":"/home/guagua/rustclaw/rustclaw.service"}],"include_missing":true}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.resolved_intent =
            "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_hint = "rustclaw.service".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"path_batch_facts","paths":["/home/guagua/rustclaw/rustclaw.service"]}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn missing_path_batch_facts_existence_contract_stops_before_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.round_no = 1;
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"path":"plan/missing.md"}],"include_missing":true}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.resolved_intent =
            "Read plan/missing.md; if it is absent, search plan for related markdown files"
                .to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
        route.output_contract.locator_hint = "plan/missing.md".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"path_batch_facts","paths":["plan/missing.md"]}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn missing_path_batch_facts_content_contract_continues_for_possible_fallback() {
        let mut loop_state = LoopState::new(2);
        loop_state.round_no = 1;
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","count":1,"facts":[{"error":"not found","exists":false,"path":"plan/missing.md"}],"include_missing":true}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.resolved_intent =
            "Read plan/missing.md; if it is absent, search plan for related markdown files"
                .to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
        route.output_contract.locator_hint = "plan/missing.md".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"path_batch_facts","paths":["plan/missing.md"]}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn structured_keys_free_output_can_stop_before_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"structured_keys","path":"/tmp/package.json","resolved_path":"/tmp/package.json","field_path":"scripts","exists":true,"container_type":"object","count":3,"keys":["build","dev","lint"]}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.route_reason = "llm_contract:generic_explicit_path_structured_keys".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/package.json".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"structured_keys","path":"/tmp/package.json","field_path":"scripts"}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn extract_fields_free_output_can_stop_before_second_round() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_fields","path":"/tmp/config.toml","resolved_path":"/tmp/config.toml","count":2,"results":[{"field_path":"database.sqlite_path","exists":true,"value_type":"string","value_text":"data/rustclaw.db","value":"data/rustclaw.db"},{"field_path":"tools.allow_sudo","exists":true,"value_type":"bool","value_text":"true","value":true}]}"#,
        ));
        let mut route = route_result(OutputResponseShape::Free);
        route.route_reason = "llm_contract:generic_explicit_path_extract_fields".to_string();
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "/tmp/config.toml".to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: json!({"action":"extract_fields","path":"/tmp/config.toml","field_paths":["database.sqlite_path","tools.allow_sudo"]}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn health_check_scalar_summary_continues_to_synthesis() {
        let mut loop_state = LoopState::new(2);
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "health_check",
            r#"{"clawd_process_count":1,"telegramd_process_count":0,"clawd_health_port_open":true,"clawd_log":{"exists":true,"keyword_error_count":0},"telegramd_log":{"exists":false},"system_health":{"os_family":"macos","warnings":[]}}"#,
        ));
        let mut route = route_result(OutputResponseShape::Scalar);
        route.resolved_intent =
            "执行基础健康检查，仅提取并返回操作系统相关的关键字段，排除 RustClaw 自身的状态摘要"
                .to_string();
        let actions = vec![AgentAction::CallSkill {
            skill: "health_check".to_string(),
            args: json!({}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn recipe_waiting_for_validation_does_not_stop_on_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            validation_required: true,
            saw_mutation: true,
            ..Default::default()
        };
        loop_state.has_tool_or_skill_output = true;
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "configuration updated\n",
        ));
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command":"cat ./config.json"}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Free)),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn recipe_inspect_stage_does_not_stop_on_observed_output() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Inspect,
            inspect_first: true,
            validation_required: true,
            ..Default::default()
        };
        loop_state.has_tool_or_skill_output = true;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "index.html\n"));
        let actions = vec![AgentAction::CallSkill {
            skill: "list_dir".to_string(),
            args: json!({"path":"document/nl_ops_http_demo"}),
        }];
        assert!(!should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Scalar)),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn recipe_done_does_not_scan_user_text_for_success_marker() {
        let mut loop_state = LoopState::new(2);
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Done,
            inspect_first: true,
            validation_required: true,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: true,
            ..Default::default()
        };
        loop_state.has_tool_or_skill_output = true;
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "ops-demo-ok\n"));
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command":"curl -s http://127.0.0.1:52752/ | grep -o ops-demo-ok"}),
        }];
        assert!(should_stop_for_observed_finalize(
            Some(&AgentRunContext {
                route_result: Some(route_result(OutputResponseShape::Scalar)),
                user_request: Some(
                    "验证通过时请明确输出 VALIDATION_PASSED，然后直接结束。".to_string()
                ),
                ..Default::default()
            }),
            &loop_state,
            &actions,
        ));
    }

    #[test]
    fn recoverable_recipe_failure_continues_next_round_and_keeps_repair_count() {
        let task = test_task();
        let policy = test_policy();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Repair,
            inspect_first: true,
            validation_required: true,
            max_repairs: 3,
            repair_count: 1,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };
        let outcome = RoundOutcome {
            executed_actions: 1,
            had_error: false,
            stop_signal: Some("recoverable_failure_continue_round".to_string()),
            next_goal_hint: Some("repair sing-box".to_string()),
            no_progress: false,
        };
        assert!(!evaluate_round_outcome(
            &task,
            &mut loop_state,
            &policy,
            &outcome
        ));
        assert_eq!(loop_state.execution_recipe.repair_count, 1);
        assert_eq!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Repair
        );
        assert_eq!(loop_state.consecutive_no_progress, 0);
    }

    #[test]
    fn recoverable_failure_at_round_cap_extends_loop_once() {
        let task = test_task();
        let mut policy = test_policy();
        policy.max_rounds = 2;
        policy.recoverable_failure_extra_rounds = 1;
        let mut loop_state = LoopState::new(2);
        loop_state.round_no = 2;
        let outcome = RoundOutcome {
            executed_actions: 1,
            had_error: false,
            stop_signal: Some("recoverable_failure_continue_round".to_string()),
            next_goal_hint: Some("try alternate locator".to_string()),
            no_progress: false,
        };

        assert!(!evaluate_round_outcome(
            &task,
            &mut loop_state,
            &policy,
            &outcome
        ));
        assert_eq!(loop_state.max_rounds, 3);
        assert_eq!(loop_state.recoverable_failure_extra_rounds_used, 1);
    }

    #[test]
    fn recoverable_failure_extra_round_exhaustion_stops() {
        let task = test_task();
        let mut policy = test_policy();
        policy.max_rounds = 2;
        policy.recoverable_failure_extra_rounds = 1;
        let mut loop_state = LoopState::new(2);
        loop_state.round_no = 2;
        loop_state.recoverable_failure_extra_rounds_used = 1;
        let outcome = RoundOutcome {
            executed_actions: 1,
            had_error: false,
            stop_signal: Some("recoverable_failure_continue_round".to_string()),
            next_goal_hint: Some("try alternate locator".to_string()),
            no_progress: false,
        };

        assert!(evaluate_round_outcome(
            &task,
            &mut loop_state,
            &policy,
            &outcome
        ));
        assert_eq!(loop_state.max_rounds, 2);
        assert_eq!(loop_state.recoverable_failure_extra_rounds_used, 1);
    }

    #[test]
    fn exhausted_recipe_budget_stops_next_round() {
        let task = test_task();
        let policy = test_policy();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 2;
        loop_state.execution_recipe = ExecutionRecipeRuntimeState {
            kind: ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Repair,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            repair_count: 3,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };
        let outcome = RoundOutcome {
            executed_actions: 1,
            had_error: false,
            stop_signal: Some("recipe_repair_budget_exhausted".to_string()),
            next_goal_hint: None,
            no_progress: false,
        };
        assert!(evaluate_round_outcome(
            &task,
            &mut loop_state,
            &policy,
            &outcome
        ));
        assert_eq!(loop_state.execution_recipe.repair_count, 3);
        assert_eq!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Repair
        );
    }

    #[test]
    fn explicit_execution_recipe_hint_takes_priority_over_local_detection() {
        let spec = initial_execution_recipe_spec(
            "configure sing-box and verify the proxy works",
            "configure sing-box and verify the proxy works",
            Some(&AgentRunContext {
                execution_recipe_hint: Some(ExecutionRecipeSpec {
                    kind: ExecutionRecipeKind::OpsClosedLoop,
                    profile: ExecutionRecipeProfile::CodeChange,
                    target_scope: ExecutionRecipeTargetScope::Greenfield,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                }),
                route_result: Some(route_result(OutputResponseShape::Free)),
                user_request: Some("configure sing-box and verify the proxy works".to_string()),
                ..Default::default()
            }),
        );
        assert_eq!(spec.profile, ExecutionRecipeProfile::CodeChange);
        assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::Greenfield);
    }
}

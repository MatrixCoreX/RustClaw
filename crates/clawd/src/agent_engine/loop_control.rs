use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tracing::{info, warn};

use super::support::publish_agent_loop_checkpoint_progress;
use super::{
    append_progress_hint, attempt_ledger, encode_progress_i18n, ensure_task_running,
    execute_actions_once, load_agent_loop_guard_policy, prepare_round_actions, push_round_trace,
    AgentLoopGuardPolicy, AgentRunContext, LoopState, RoundOutcome,
};
use crate::{AgentAction, AppState, AskReply, ClaimedTask, IntentOutputContract};

#[path = "loop_control_answer_recovery.rs"]
mod loop_control_answer_recovery;
#[path = "loop_control_answer_recovery_parse.rs"]
mod loop_control_answer_recovery_parse;
#[path = "loop_control_answer_recovery_text.rs"]
mod loop_control_answer_recovery_text;
#[path = "loop_control_filesystem_mutation_recovery.rs"]
mod loop_control_filesystem_mutation_recovery;
#[path = "loop_control_finalization_gate.rs"]
mod loop_control_finalization_gate;
#[path = "loop_control_local_health_recovery.rs"]
mod loop_control_local_health_recovery;
#[path = "loop_control_machine_status_gap.rs"]
mod loop_control_machine_status_gap;
#[path = "loop_control_observe_round.rs"]
mod loop_control_observe_round;
#[path = "loop_control_post_write_evidence_guard.rs"]
mod loop_control_post_write_evidence_guard;
#[path = "loop_control_recent_artifacts_recovery.rs"]
mod loop_control_recent_artifacts_recovery;
#[path = "loop_control_structured_clarify.rs"]
mod loop_control_structured_clarify;
#[path = "loop_control_verifier_retry_commit.rs"]
mod loop_control_verifier_retry_commit;

use loop_control_answer_recovery::*;
use loop_control_answer_recovery_parse::*;
use loop_control_answer_recovery_text::*;
use loop_control_filesystem_mutation_recovery::*;
use loop_control_finalization_gate::*;
use loop_control_local_health_recovery::*;
use loop_control_machine_status_gap::*;
pub(in crate::agent_engine) use loop_control_observe_round::observation_round_needs_planner;
use loop_control_observe_round::{
    observe_only_round_should_continue, read_observe_round_should_continue,
};
use loop_control_post_write_evidence_guard::*;
use loop_control_recent_artifacts_recovery::*;
use loop_control_structured_clarify::*;
use loop_control_verifier_retry_commit::verifier_retry_answer_has_required_machine_evidence;

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

fn answer_verifier_summary_to_out(
    verifier: &crate::task_journal::TaskJournalAnswerVerifierSummary,
) -> crate::answer_verifier::AnswerVerifierOut {
    crate::answer_verifier::AnswerVerifierOut {
        pass: verifier.pass,
        missing_evidence_fields: verifier.missing_evidence_fields.clone(),
        answer_incomplete_reason: verifier.answer_incomplete_reason.clone(),
        should_retry: verifier.should_retry,
        retry_instruction: verifier.retry_instruction.clone(),
        confidence: verifier.confidence,
    }
    .normalized()
}

fn commit_answer_verifier_retry_answer(reply: &mut AskReply, retried_answer: String) -> bool {
    if !verifier_retry_answer_has_required_machine_evidence(reply, &retried_answer) {
        info!("answer_verifier_retry_commit_rejected_missing_machine_validation_evidence");
        return false;
    }
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(retried_answer.clone());
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&retried_answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        journal.record_final_stop_signal(
            crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL,
        );
    }
    reply.text = retried_answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = true;
    true
}

async fn record_session_start_hooks(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
) {
    let session_start = crate::agent_hooks::lifecycle_stage_outcome_for_state(
        state,
        &task.task_id,
        crate::agent_hooks::HookStage::SessionStart,
        "agent_loop.session_start",
        json!({
            "task_kind": task.kind,
            "task_channel": task.channel,
        }),
    )
    .await;
    loop_state
        .task_observations
        .extend(session_start.machine_observations("agent_loop"));

    let prompt_submit = crate::agent_hooks::lifecycle_stage_outcome_for_state(
        state,
        &task.task_id,
        crate::agent_hooks::HookStage::UserPromptSubmit,
        "agent_loop.user_prompt_submit",
        json!({
            "input_char_count": user_text.chars().count(),
            "input_byte_count": user_text.len(),
        }),
    )
    .await;
    loop_state
        .task_observations
        .extend(prompt_submit.machine_observations("agent_loop"));
}

fn terminal_user_answer_stop_signal(loop_state: &LoopState) -> Option<&'static str> {
    has_authoritative_delivery(loop_state).then_some("terminal_user_answer_ready")
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

fn route_expects_terminal_user_answer(route_result: &IntentOutputContract) -> bool {
    if route_result.delivery_required {
        return false;
    }
    !matches!(
        route_result.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    )
}

fn route_requires_direct_candidate_for_observed_stop(route_result: &IntentOutputContract) -> bool {
    route_result.semantic_kind_is(crate::OutputSemanticKind::ServiceStatus)
        && crate::evidence_policy::final_answer_shape_for_output_contract(route_result)
            .is_some_and(|shape| shape.allows_model_language())
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
                | AgentAction::CallCapability { .. }
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

fn route_needs_workspace_text_evidence_before_observed_finalize(
    route: &IntentOutputContract,
) -> bool {
    route.requires_content_evidence
        && !route.delivery_required
        && route.response_shape == crate::OutputResponseShape::Free
        && route.semantic_kind_is_unclassified()
        && route.locator_kind == crate::OutputLocatorKind::CurrentWorkspace
        && route.locator_hint.trim().is_empty()
}

fn structured_scalar_equality_observation_can_finalize(
    route_result: &IntentOutputContract,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    route_result.semantic_kind_is(crate::OutputSemanticKind::RecentScalarEqualityCheck)
        && !route_result.delivery_required
        && has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && super::observed_output::structured_scalar_equality_direct_answer(
            None,
            route_result,
            loop_state,
            None,
        )
        .is_some()
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
    super::observed_output::extract_answer_from_observed_output(loop_state, agent_run_context)
        .is_some_and(|answer| text_has_exact_marker_line(&answer, marker))
        || super::observed_output::extract_direct_scalar_from_generic_output(
            loop_state,
            agent_run_context,
        )
        .is_some_and(|answer| text_has_exact_marker_line(&answer, marker))
}

fn text_has_exact_marker_line(text: &str, marker: &str) -> bool {
    let marker = marker.trim();
    !marker.is_empty() && text.lines().map(str::trim).any(|line| line == marker)
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
    let route_result = agent_run_context.and_then(|ctx| ctx.output_contract());
    let Some(route_result) = route_result else {
        return false;
    };
    let output_contract = route_result.clone();
    if false || !loop_state.has_tool_or_skill_output || has_authoritative_delivery(loop_state) {
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
        super::observed_output::extract_answer_from_observed_output(loop_state, agent_run_context)
            .is_some();
    let has_observed_stop_candidate =
        if route_requires_direct_candidate_for_observed_stop(route_result) {
            has_direct_observed_answer
        } else {
            super::observed_output::has_observed_answer_candidates(loop_state)
        };
    if read_observe_round_should_continue(&output_contract, loop_state, actions) {
        return false;
    }
    if observation_round_needs_planner(&output_contract, loop_state, actions)
        && !has_observed_stop_candidate
    {
        return false;
    }
    if structured_scalar_equality_observation_can_finalize(route_result, loop_state, actions) {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.semantic_kind_is(crate::OutputSemanticKind::ExistenceWithPath)
        && has_direct_observed_answer
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if has_executable_observation_or_action(actions)
        && !has_discussion_followup_action(actions)
        && recent_artifacts_inventory_observation_can_finalize(route_result, loop_state)
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.semantic_kind_is(crate::OutputSemanticKind::RecentArtifactsJudgment)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if quantity_comparison_one_sentence_needs_model_language_before_stop(route_result)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if super::observed_output::route_disallows_direct_observation_passthrough(route_result)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if route_result.response_shape != crate::OutputResponseShape::Scalar
        && loop_state.round_no < loop_state.max_rounds
        && latest_path_batch_facts_all_missing(loop_state)
        && !has_discussion_followup_action(actions)
    {
        return false;
    }
    if has_direct_observed_answer
        && route_result.response_shape != crate::OutputResponseShape::Scalar
    {
        return required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        });
    }
    if route_result.response_shape == crate::OutputResponseShape::Scalar {
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
        ) && super::observed_output::extract_answer_from_observed_output(
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
        && has_observed_stop_candidate;
    can_stop
        && required_success_marker.is_none_or(|marker| {
            observed_answer_contains_required_success_marker(agent_run_context, loop_state, marker)
        })
}

fn quantity_comparison_one_sentence_needs_model_language_before_stop(
    route_result: &IntentOutputContract,
) -> bool {
    route_result.semantic_kind_is(crate::OutputSemanticKind::QuantityComparison)
        && route_result.response_shape == crate::OutputResponseShape::OneSentence
        && crate::evidence_policy::final_answer_shape_for_output_contract(route_result)
            .is_some_and(|shape| shape.allows_model_language())
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

fn soft_budget_checkpoint_resume_reason(
    loop_state: &LoopState,
    policy: &AgentLoopGuardPolicy,
    outcome: &RoundOutcome,
) -> Option<&'static str> {
    if outcome.had_error {
        return None;
    }
    if outcome
        .stop_signal
        .as_deref()
        .is_some_and(|signal| signal == "recoverable_failure_continue_round")
    {
        if let Some(reason) = recoverable_provider_blocker_resume_reason(loop_state) {
            return Some(reason);
        }
        return None;
    }
    if outcome.stop_signal.is_some() || outcome.executed_actions == 0 {
        return None;
    }
    if outcome.no_progress && loop_state.consecutive_no_progress > policy.no_progress_limit {
        return Some("agent_loop_no_progress_limit");
    }
    if policy.multi_round_enabled && loop_state.round_no >= loop_state.max_rounds {
        return Some("agent_loop_max_rounds");
    }
    None
}

fn recoverable_provider_blocker_resume_reason(loop_state: &LoopState) -> Option<&'static str> {
    let latest = loop_state
        .attempt_ledger_entries
        .iter()
        .rev()
        .find(|entry| {
            entry.provider_status.is_some()
                && entry.recovery_action.trim() == "wait_background"
                && entry.status.trim() != crate::executor::StepExecutionStatus::Ok.as_str()
        })?;
    latest.provider_status.as_ref()?;
    Some("provider_blocker_wait_background")
}

fn worker_soft_checkpoint_after_seconds(worker_timeout_secs: u64) -> Option<u64> {
    let timeout = worker_timeout_secs.max(1);
    if timeout <= 2 {
        return None;
    }
    let reserve = (timeout / 10).clamp(1, 30);
    let soft_after = timeout.saturating_sub(reserve);
    (soft_after > 0 && soft_after < timeout).then_some(soft_after)
}

fn worker_soft_checkpoint_after(worker_timeout_secs: u64) -> Option<Duration> {
    worker_soft_checkpoint_after_seconds(worker_timeout_secs).map(Duration::from_secs)
}

fn worker_budget_near_exhaustion(
    started_at: Instant,
    soft_checkpoint_after: Option<Duration>,
) -> bool {
    soft_checkpoint_after.is_some_and(|duration| started_at.elapsed() >= duration)
}

fn loop_state_has_recoverable_checkpoint_state(loop_state: &LoopState) -> bool {
    loop_state.task_checkpoint.is_some()
        || !loop_state.executed_step_results.is_empty()
        || !loop_state.successful_action_fingerprints.is_empty()
        || loop_state.has_tool_or_skill_output
}

async fn run_agent_round(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &mut AgentLoopGuardPolicy,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    initial_plan: Option<&crate::PlanResult>,
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
        initial_plan,
    )
    .await?;
    push_round_trace(loop_state, goal, &prepared_round);
    crate::task_event_transport::publish_loop_state_snapshot(state, task, user_text, loop_state);
    record_agent_loop_decision_envelope_output_vars(loop_state, &prepared_round.plan_result);
    if !loop_state
        .output_vars
        .contains_key("agent_loop.planner_budget_profile")
    {
        let profile = AgentLoopGuardPolicy::budget_profile_for_context(
            loop_state.execution_recipe,
            prepared_round.effective_output_contract.as_ref(),
        );
        *policy = policy.adjusted_for_context(
            loop_state.execution_recipe,
            prepared_round.effective_output_contract.as_ref(),
        );
        loop_state.max_rounds = policy.max_rounds.max(1);
        loop_state.output_vars.insert(
            "agent_loop.planner_budget_profile".to_string(),
            profile.as_str().to_string(),
        );
        info!(
            "loop_planner_budget_profile task_id={} profile={} max_rounds={} max_steps={} max_tool_calls={} no_progress_limit={}",
            task.task_id,
            profile.as_str(),
            policy.max_rounds,
            policy.max_steps,
            policy.max_tool_calls,
            policy.no_progress_limit
        );
    }
    if let Some(output_contract) = prepared_round.effective_output_contract.as_ref() {
        loop_state.output_contract = Some(output_contract.clone());
        if let Some(final_answer_shape) =
            crate::evidence_policy::final_answer_shape_for_output_contract(output_contract)
        {
            loop_state.output_vars.insert(
                "agent_loop.final_answer_shape".to_string(),
                final_answer_shape.as_str().to_string(),
            );
            loop_state.output_vars.insert(
                "agent_loop.final_answer_shape_class".to_string(),
                final_answer_shape.class().as_str().to_string(),
            );
        }
    }
    if let Some(intent) = structured_respond_terminal_intent_from_plan(&prepared_round.plan_result)
        .filter(|intent| intent.terminal_intent == "clarify")
        .filter(|_| actions_allow_structured_respond_terminal_intent(&prepared_round.actions))
        .or_else(|| {
            structured_respond_terminal_intent_from_boundary_observation_clarify(
                loop_state,
                &prepared_round.actions,
            )
        })
    {
        if let Some(outcome) = try_recover_inconsistent_boundary_clarify(
            loop_state,
            prepared_round.effective_output_contract.as_ref(),
            &intent,
        ) {
            info!(
                "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
                task.task_id,
                loop_state.round_no,
                outcome.executed_actions,
                outcome.no_progress,
                outcome.stop_signal.as_deref().unwrap_or(""),
                crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
            );
            return Ok(outcome);
        }
        let outcome = apply_structured_respond_clarify_to_loop_state(loop_state, &intent);
        info!(
            "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            outcome.executed_actions,
            outcome.no_progress,
            outcome.stop_signal.as_deref().unwrap_or(""),
            crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
        );
        return Ok(outcome);
    }
    let actions = prepared_round.actions;
    if let Some(intent) = forced_boundary_observation_clarify_intent(loop_state, &actions) {
        let outcome = apply_structured_respond_clarify_to_loop_state(loop_state, &intent);
        info!(
            "loop_round_eval task_id={} round={} executed_actions={} no_progress={} stop_signal={} next_goal_hint={}",
            task.task_id,
            loop_state.round_no,
            outcome.executed_actions,
            outcome.no_progress,
            outcome.stop_signal.as_deref().unwrap_or(""),
            crate::truncate_for_log(outcome.next_goal_hint.as_deref().unwrap_or(""))
        );
        return Ok(outcome);
    }
    loop_state.verified_action_window_active =
        prepared_round.verify_result.approved && !actions.is_empty();
    let execute_result = execute_actions_once(
        state,
        task,
        goal,
        user_text,
        &actions,
        loop_state,
        policy,
        agent_run_context,
    )
    .await;
    loop_state.verified_action_window_active = false;
    let mut outcome = execute_result?;
    if outcome.stop_signal.is_none() {
        if let Some(stop_signal) = terminal_user_answer_stop_signal(loop_state) {
            outcome.stop_signal = Some(stop_signal.to_string());
        }
    }
    if outcome.stop_signal.is_none()
        && should_stop_for_observed_finalize(agent_run_context, loop_state, &actions)
    {
        outcome.stop_signal = Some("observed_output_ready".to_string());
    }
    if outcome.stop_signal.is_none()
        && prepared_round
            .effective_output_contract
            .as_ref()
            .is_some_and(|contract| {
                observe_only_round_should_continue(contract, loop_state, &actions)
            })
    {
        loop_state.has_recoverable_failure_context = true;
        loop_state.output_vars.insert(
            "agent_loop.observe_only_continue".to_string(),
            "true".to_string(),
        );
        outcome.stop_signal = Some("recoverable_failure_continue_round".to_string());
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
            .and_then(|ctx| ctx.output_contract())
            .is_some(),
        agent_run_context
            .and_then(|ctx| ctx.user_request.as_deref())
            .is_some_and(|text| !text.trim().is_empty())
    );
    crate::execution_recipe::ExecutionRecipeSpec::default()
}

pub(super) async fn run_agent_with_loop_with_initial_observations(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    initial_task_observations: &[Value],
) -> Result<AskReply, String> {
    run_agent_with_loop_seeded_and_initial_plan(
        state,
        task,
        goal,
        user_text,
        agent_run_context,
        None,
        None,
        initial_task_observations,
    )
    .await
}

pub(super) async fn run_agent_with_loop_seeded(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    resume_checkpoint: Option<&crate::task_lifecycle::TaskCheckpoint>,
) -> Result<AskReply, String> {
    run_agent_with_loop_seeded_and_initial_plan(
        state,
        task,
        goal,
        user_text,
        agent_run_context,
        resume_checkpoint,
        None,
        &[],
    )
    .await
}

pub(super) async fn run_agent_with_loop_direct_plan(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    initial_plan: &crate::PlanResult,
) -> Result<AskReply, String> {
    run_agent_with_loop_seeded_and_initial_plan(
        state,
        task,
        goal,
        user_text,
        agent_run_context,
        None,
        Some(initial_plan),
        &[],
    )
    .await
}

async fn run_agent_with_loop_seeded_and_initial_plan(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    resume_checkpoint: Option<&crate::task_lifecycle::TaskCheckpoint>,
    initial_plan: Option<&crate::PlanResult>,
    initial_task_observations: &[Value],
) -> Result<AskReply, String> {
    let base_policy = load_agent_loop_guard_policy(state);
    let mut loop_state = LoopState::new(base_policy.max_rounds.max(1));
    super::seed_loop_state_for_agent_run(&mut loop_state, agent_run_context, resume_checkpoint);
    loop_state
        .task_observations
        .extend(initial_task_observations.iter().cloned());
    record_session_start_hooks(state, task, user_text, &mut loop_state).await;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        initial_execution_recipe_spec(goal, user_text, agent_run_context),
    );
    let budget_profile =
        AgentLoopGuardPolicy::budget_profile_for_context(loop_state.execution_recipe, None);
    let mut policy = base_policy.adjusted_for_context(loop_state.execution_recipe, None);
    loop_state.max_rounds = policy.max_rounds.max(1);
    base_policy.apply_recipe_runtime_overrides(&mut loop_state.execution_recipe);
    let enabled_rollout_switches = policy.enabled_rollout_switches();
    if !enabled_rollout_switches.is_empty() {
        loop_state.output_vars.insert(
            "rollout_switches_enabled".to_string(),
            enabled_rollout_switches.join(","),
        );
    }
    info!(
        "loop_budget_profile task_id={} profile={} max_rounds={} max_steps={} max_tool_calls={} no_progress_limit={} repeat_action_limit={}",
        task.task_id,
        budget_profile.as_str(),
        policy.max_rounds,
        policy.max_steps,
        policy.max_tool_calls,
        policy.no_progress_limit,
        policy.repeat_action_limit
    );
    let mut round = 1usize;
    let mut answer_verifier_retry_count = 0usize;
    let loop_started_at = Instant::now();
    let worker_soft_checkpoint_after =
        worker_soft_checkpoint_after(state.worker.worker_task_timeout_seconds);
    loop {
        while round <= loop_state.max_rounds {
            ensure_task_running(state, task)?;
            loop_state.round_no = round;
            if worker_budget_near_exhaustion(loop_started_at, worker_soft_checkpoint_after)
                && loop_state_has_recoverable_checkpoint_state(&loop_state)
            {
                loop_state.last_stop_signal = Some("budget_near_exhaustion".to_string());
                publish_agent_loop_checkpoint_progress(
                    state,
                    task,
                    &mut loop_state,
                    "budget_near_exhaustion",
                );
                break;
            }
            super::maybe_publish_execution_recipe_phase_hint(state, task, &mut loop_state);
            let outcome = run_agent_round(
                state,
                task,
                goal,
                user_text,
                &mut policy,
                &mut loop_state,
                agent_run_context,
                (round == 1).then_some(initial_plan).flatten(),
            )
            .await?;
            loop_state.last_stop_signal = outcome.stop_signal.clone();
            if worker_budget_near_exhaustion(loop_started_at, worker_soft_checkpoint_after)
                && !outcome.had_error
                && outcome.executed_actions > 0
                && loop_state_has_recoverable_checkpoint_state(&loop_state)
            {
                loop_state.last_stop_signal = Some("budget_near_exhaustion".to_string());
                publish_agent_loop_checkpoint_progress(
                    state,
                    task,
                    &mut loop_state,
                    "budget_near_exhaustion",
                );
                break;
            }
            if evaluate_round_outcome(task, &mut loop_state, &policy, &outcome) {
                if let Some(resume_reason) =
                    soft_budget_checkpoint_resume_reason(&loop_state, &policy, &outcome)
                {
                    publish_agent_loop_checkpoint_progress(
                        state,
                        task,
                        &mut loop_state,
                        resume_reason,
                    );
                }
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
        let answer_contract = answer_contract_for_reply(user_text, &reply);
        promote_local_code_projection_from_machine_evidence_for_verifier_candidate(
            &mut reply,
            user_text,
            &pre_finalize_loop_state,
            agent_run_context,
        );
        promote_publishable_strict_json_projection_for_verifier_candidate(
            &mut reply,
            answer_contract.as_ref(),
            &pre_finalize_loop_state,
        );
        prefer_terminal_model_answer_for_verifier_candidate(&mut reply, answer_contract.as_ref());
        enforce_post_write_content_evidence_guard(&mut reply);
        enforce_code_mutation_validation_success_guard(&mut reply);
        let mut pre_verifier_recovery_loop_state = pre_finalize_loop_state.clone();
        if try_run_post_write_validation_reserve_recovery(
            state,
            task,
            goal,
            user_text,
            &policy,
            &mut pre_verifier_recovery_loop_state,
            &reply,
            agent_run_context,
        )
        .await?
        {
            loop_state = pre_verifier_recovery_loop_state;
            round = loop_state.max_rounds + 1;
            continue;
        }
        attach_answer_verifier_if_missing(
            state,
            task,
            user_text,
            answer_contract.as_ref(),
            &mut reply,
        )
        .await;
        enforce_post_write_content_evidence_guard(&mut reply);
        enforce_code_mutation_validation_success_guard(&mut reply);
        let route_result = answer_contract.as_ref();
        suppress_answer_verifier_retry_if_structurally_satisfied(&mut reply, route_result);
        suppress_answer_verifier_retry_if_user_locator_disambiguation(&mut reply, route_result);
        suppress_answer_verifier_retry_if_confirmed_missing_file_delivery(&mut reply, route_result);
        if try_preserve_rss_source_hosts_from_structured_evidence(&mut reply) {
            return Ok(reply);
        }
        if try_recover_document_heading_answer_verifier_gap(route_result, &mut reply) {
            return Ok(reply);
        }
        if let Some(verifier) = answer_verifier_retry_summary(&reply, route_result).cloned() {
            if answer_verifier_output_format_machine_payload_gap(&verifier, &reply.text) {
                if let (Some(route), Some(journal_snapshot)) =
                    (route_result, reply.task_journal.clone())
                {
                    let verifier_out = answer_verifier_summary_to_out(&verifier);
                    if let Some(retried_answer) = crate::finalize::retry_loop_answer_after_verifier(
                        state,
                        task,
                        user_text,
                        &journal_snapshot,
                        &reply.text,
                        &verifier_out,
                    )
                    .await
                    {
                        let retry_verifier = crate::answer_verifier::verify_answer_observe_only(
                            state,
                            task,
                            user_text,
                            route,
                            &journal_snapshot,
                            &retried_answer,
                        )
                        .await;
                        if let Some(retry_verifier) = retry_verifier {
                            if retry_verifier_accepts_rewritten_answer(
                                &retry_verifier,
                                &retried_answer,
                            ) {
                                if commit_answer_verifier_retry_answer(&mut reply, retried_answer) {
                                    info!(
                                        "answer_verifier_machine_payload_rewritten_to_visible_answer"
                                    );
                                    return Ok(reply);
                                }
                            }
                            if let Some(journal) = reply.task_journal.as_mut() {
                                journal.record_answer_verifier_summary(retry_verifier);
                            }
                        } else if retry_rewritten_answer_is_publishable(&retried_answer) {
                            if commit_answer_verifier_retry_answer(&mut reply, retried_answer) {
                                info!(
                                    "answer_verifier_machine_payload_rewritten_to_visible_answer"
                                );
                                return Ok(reply);
                            }
                        } else {
                            info!(
                                "answer_verifier_retry_rewritten_answer_unpublishable_unresolved_machine_fields"
                            );
                        }
                    }
                }
            }
            if try_recover_structured_listing_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_local_health_answer_verifier_gap_from_loop_state(
                route_result,
                &pre_finalize_loop_state,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_machine_kv_summary_output_format_answer_verifier_gap(
                route_result,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if answer_verifier_gap_requests_observed_content_rewrite(&verifier)
                && try_rewrite_answer_verifier_gap_with_observed_evidence(
                    state,
                    task,
                    user_text,
                    route_result,
                    &verifier,
                    &mut reply,
                )
                .await
            {
                return Ok(reply);
            }
            let mut deterministic_recovery_loop_state = pre_finalize_loop_state.clone();
            if try_run_post_write_validation_reserve_recovery(
                state,
                task,
                goal,
                user_text,
                &policy,
                &mut deterministic_recovery_loop_state,
                &reply,
                agent_run_context,
            )
            .await?
            {
                loop_state = deterministic_recovery_loop_state;
                round = loop_state.max_rounds + 1;
                continue;
            }
            if answer_verifier_retry_budget_available(&policy, answer_verifier_retry_count) {
                loop_state = pre_finalize_loop_state.clone();
                if try_run_post_write_content_evidence_readback_recovery(
                    state,
                    task,
                    goal,
                    user_text,
                    &policy,
                    &mut loop_state,
                    &reply,
                    agent_run_context,
                )
                .await?
                {
                    round = loop_state.max_rounds + 1;
                    continue;
                }
            }
            if answer_verifier_retry_budget_available(&policy, answer_verifier_retry_count) {
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
            if try_recover_structured_listing_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_latest_synthesis_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_log_analyze_answer_verifier_gap(user_text, &mut reply) {
                return Ok(reply);
            }
            if try_recover_structured_count_answer_verifier_gap(route_result, user_text, &mut reply)
            {
                return Ok(reply);
            }
            if try_recover_structured_search_answer_verifier_gap(
                route_result,
                user_text,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_rss_news_answer_verifier_gap(&mut reply) {
                return Ok(reply);
            }
            if try_recover_document_heading_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_structured_scalar_output_format_answer_verifier_gap(
                route_result,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_recover_machine_kv_summary_output_format_answer_verifier_gap(
                route_result,
                &mut reply,
            ) {
                return Ok(reply);
            }
            if try_rewrite_answer_verifier_gap_with_observed_evidence(
                state,
                task,
                user_text,
                route_result,
                &verifier,
                &mut reply,
            )
            .await
            {
                return Ok(reply);
            }
            if try_recover_structured_evidence_table_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_http_health_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_local_health_answer_verifier_gap(route_result, &mut reply) {
                return Ok(reply);
            }
            if try_recover_recent_artifacts_answer_verifier_gap(route_result, &mut reply) {
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
            if try_recover_filesystem_mutation_success_answer_verifier_gap(route_result, &mut reply)
            {
                return Ok(reply);
            }
            if try_accept_language_only_output_format_answer_verifier_gap(route_result, &mut reply)
            {
                return Ok(reply);
            }
            mark_reply_failed_after_answer_verifier_exhausted(user_text, &mut reply, &verifier);
        }
        return Ok(reply);
    }
}

#[cfg(test)]
#[path = "loop_control_tests.rs"]
mod tests;

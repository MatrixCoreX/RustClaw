use tracing::info;

use super::{
    dispatch_round_action, ensure_task_running, plan_step_label, ActionLoopDecision,
    AgentLoopGuardPolicy, AgentRunContext, LoopState, RoundOutcome,
};
use crate::{AgentAction, AppState, ClaimedTask};
use serde_json::Value;
use std::collections::BTreeSet;

struct RoundProgressSnapshot {
    delivery_count: usize,
    machine_progress_fingerprints: BTreeSet<String>,
}

fn capture_round_progress_snapshot(loop_state: &LoopState) -> RoundProgressSnapshot {
    RoundProgressSnapshot {
        delivery_count: loop_state.delivery_messages.len(),
        machine_progress_fingerprints: super::progress_contract::machine_progress_fingerprints(
            loop_state,
        ),
    }
}

fn finalize_execute_round_outcome(
    loop_state: &LoopState,
    snapshot: &RoundProgressSnapshot,
    actionable_count: usize,
    executed_actions: usize,
    ended_with_user_visible_output: bool,
    mut stop_signal: Option<String>,
) -> RoundOutcome {
    if stop_signal.is_none()
        && executed_actions == actionable_count
        && ended_with_user_visible_output
    {
        stop_signal = Some("plan_exhausted_user_visible".to_string());
    }
    let delivery_grew = loop_state.delivery_messages.len() > snapshot.delivery_count;
    let machine_progress = super::progress_contract::machine_progress_fingerprints(loop_state);
    let no_progress =
        !delivery_grew && machine_progress.is_subset(&snapshot.machine_progress_fingerprints);
    RoundOutcome {
        executed_actions,
        had_error: false,
        stop_signal,
        next_goal_hint: loop_state.delivery_messages.last().cloned(),
        no_progress,
    }
}

fn repeated_successful_action_is_allowed_for_active_recipe(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
) -> bool {
    let Some(effect) = action_effect_for_repeat_guard(state, loop_state, action) else {
        return false;
    };
    action_effect_is_repeatable_for_active_recipe(loop_state.execution_recipe, effect)
        || waiting_task_allows_repeated_observation(loop_state, effect)
}

fn action_effect_for_repeat_guard(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
) -> Option<crate::execution_recipe::ActionEffect> {
    let (skill_name, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        AgentAction::CallCapability { .. } => {
            let resolved =
                crate::capability_resolver::resolve_agent_action_for_state(state, action.clone());
            if matches!(resolved, AgentAction::CallCapability { .. }) {
                return None;
            }
            return action_effect_for_repeat_guard(state, loop_state, &resolved);
        }
        AgentAction::SynthesizeAnswer { .. } => return None,
        AgentAction::Respond { .. } | AgentAction::Think { .. } => return None,
    };
    let normalized_skill = state.resolve_canonical_skill_name(skill_name);
    let raw_effect =
        crate::execution_recipe::classify_skill_action_effect(state, &normalized_skill, args);
    Some(crate::execution_recipe::effective_action_effect_for_recipe(
        loop_state.execution_recipe,
        raw_effect,
    ))
}

fn action_effect_is_repeatable_for_active_recipe(
    recipe: crate::execution_recipe::ExecutionRecipeRuntimeState,
    effect: crate::execution_recipe::ActionEffect,
) -> bool {
    recipe.is_active()
        && !matches!(
            recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Done
        )
        && !effect.mutates
        && (effect.observes || effect.validates)
}

fn waiting_task_allows_repeated_observation(
    loop_state: &LoopState,
    effect: crate::execution_recipe::ActionEffect,
) -> bool {
    if effect.mutates || !(effect.observes || effect.validates) {
        return false;
    }
    loop_state
        .task_lifecycle
        .as_ref()
        .and_then(|value| value.get("state"))
        .and_then(Value::as_str)
        .is_some_and(|state| matches!(state, "waiting" | "background"))
        || loop_state
            .task_checkpoint
            .as_ref()
            .and_then(|value| value.get("pending_async_job"))
            .is_some_and(|job| !job.is_null())
}

fn check_repeat_action_guard(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    action: &AgentAction,
    fingerprint: &str,
    step_in_round: usize,
) -> Option<String> {
    if matches!(action, AgentAction::Respond { .. }) {
        return None;
    }
    let repeatable_observation =
        repeated_successful_action_is_allowed_for_active_recipe(state, loop_state, action);
    let repeat_count = loop_state
        .repeat_action_counts
        .entry(fingerprint.to_string())
        .or_insert(0);
    *repeat_count += 1;
    if *repeat_count > policy.repeat_action_limit && !repeatable_observation {
        if let Some(attribution) = super::registry_idempotency_guard_attribution(
            state,
            policy,
            action,
            fingerprint,
            "registry_idempotency_repeat_action_limit",
            Some(*repeat_count),
            Some(policy.repeat_action_limit),
        ) {
            loop_state.rollout_attribution.push(attribution);
        }
        info!(
            "executor_result_error task_id={} round={} step={} type=guard error={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            format!(
                "repeat action guard triggered: count={} limit={} action={}",
                *repeat_count,
                policy.repeat_action_limit,
                crate::truncate_for_log(fingerprint)
            )
        );
        return Some("repeat_action_limit".to_string());
    }
    if let Some(success_count) = loop_state.successful_action_fingerprints.get(fingerprint) {
        if repeatable_observation {
            return None;
        }
        let repeated_observation_ready = action_effect_for_repeat_guard(state, loop_state, action)
            .is_some_and(|effect| !effect.mutates && (effect.observes || effect.validates));
        let (reason_code, stop_signal) = if repeated_observation_ready {
            (
                "registry_idempotency_repeat_observation_ready",
                "structured_observation_already_ready",
            )
        } else {
            (
                "registry_idempotency_repeat_completed_action",
                "repeat_completed_action",
            )
        };
        if let Some(attribution) = super::registry_idempotency_guard_attribution(
            state,
            policy,
            action,
            fingerprint,
            reason_code,
            Some(*success_count),
            None,
        ) {
            loop_state.rollout_attribution.push(attribution);
        }
        info!(
            "executor_result_error task_id={} round={} step={} type=guard error={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            format!(
                "skip repeated successful action: count={} action={}",
                success_count,
                crate::truncate_for_log(fingerprint)
            )
        );
        return Some(stop_signal.to_string());
    }
    None
}

fn action_counts_as_tool_call(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::CallTool { .. }
            | AgentAction::CallSkill { .. }
            | AgentAction::CallCapability { .. }
    )
}

fn bare_last_output_placeholder(content: &str) -> bool {
    let trimmed = content.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return false;
    }
    let inner = trimmed[2..trimmed.len().saturating_sub(2)].trim();
    let lower = inner.to_ascii_lowercase();
    lower == "last_output" || lower.starts_with("last_output.") || lower.starts_with("last_output[")
}

fn terminal_synthesis_can_skip_remaining_actions(
    action: &AgentAction,
    remaining_actions: &[AgentAction],
    loop_state: &LoopState,
) -> bool {
    if !matches!(action, AgentAction::SynthesizeAnswer { .. }) {
        return false;
    }
    if loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        return false;
    }
    let strict_json_terminal = terminal_synthesis_strict_json_owns_response(loop_state);
    !remaining_actions.is_empty()
        && remaining_actions.iter().all(|action| match action {
            AgentAction::Think { .. } => true,
            AgentAction::Respond { content } => {
                bare_last_output_placeholder(content)
                    || (strict_json_terminal && !response_content_is_json_object(content))
            }
            AgentAction::CallSkill { .. }
            | AgentAction::CallTool { .. }
            | AgentAction::CallCapability { .. }
            | AgentAction::SynthesizeAnswer { .. } => false,
        })
}

fn terminal_synthesis_strict_json_owns_response(loop_state: &LoopState) -> bool {
    if !loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .is_some_and(response_content_is_json_object)
    {
        return false;
    }
    loop_state
        .output_vars
        .get("agent_loop.strict_json_projection_publishable")
        .is_some_and(|value| value == "true")
        || loop_state
            .output_contract
            .as_ref()
            .is_some_and(|contract| contract.response_shape == crate::OutputResponseShape::Strict)
}

fn response_content_is_json_object(content: &str) -> bool {
    serde_json::from_str::<Value>(content.trim()).is_ok_and(|value| value.is_object())
}

fn successful_structured_observation_satisfies_selector(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    current_action: &AgentAction,
    remaining_actions: &[AgentAction],
) -> bool {
    if !matches!(
        current_action,
        AgentAction::CallCapability { .. }
            | AgentAction::CallTool { .. }
            | AgentAction::CallSkill { .. }
    ) || remaining_actions.is_empty()
        || !remaining_actions.iter().all(|action| {
            matches!(
                action,
                AgentAction::SynthesizeAnswer { .. }
                    | AgentAction::Respond { .. }
                    | AgentAction::Think { .. }
            )
        })
        || loop_state.execution_recipe.needs_validation()
        || loop_state.execution_recipe.is_active()
            && !matches!(
                loop_state.execution_recipe.phase,
                crate::execution_recipe::ExecutionRecipePhase::Done
            )
    {
        return false;
    }
    latest_successful_output_satisfies_structured_selector(agent_run_context, loop_state)
}

fn latest_successful_output_satisfies_structured_selector(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
) -> bool {
    let route = loop_state
        .output_contract
        .as_ref()
        .filter(|route| {
            route
                .selection
                .structured_field_selector
                .as_deref()
                .is_some_and(|selector| !selector.trim().is_empty())
        })
        .or_else(|| agent_run_context.and_then(AgentRunContext::output_contract));
    let Some(selector) = route
        .and_then(|route| route.selection.structured_field_selector.as_deref())
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
    else {
        return false;
    };
    loop_state
        .executed_step_results
        .last()
        .filter(|step| step.is_ok())
        .and_then(|step| step.output.as_deref())
        .is_some_and(|output| {
            crate::machine_kv_projection::structured_json_satisfies_field_selector(selector, output)
        })
}

fn prior_structured_observation_satisfies_read_only_action(
    state: &AppState,
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
    action: &AgentAction,
) -> bool {
    if loop_state.execution_recipe.needs_validation()
        || loop_state.execution_recipe.is_active()
            && !matches!(
                loop_state.execution_recipe.phase,
                crate::execution_recipe::ExecutionRecipePhase::Done
            )
    {
        return false;
    }
    let Some(effect) = action_effect_for_repeat_guard(state, loop_state, action) else {
        return false;
    };
    !effect.mutates
        && (effect.observes || effect.validates)
        && latest_successful_output_satisfies_structured_selector(agent_run_context, loop_state)
}

#[allow(clippy::too_many_arguments)]
async fn try_execute_independent_read_batch(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    agent_run_context: Option<&AgentRunContext>,
    snapshot: &RoundProgressSnapshot,
    actionable_count: usize,
) -> Result<Option<RoundOutcome>, String> {
    if loop_state.execution_recipe.is_active()
        || loop_state.task_lifecycle.is_some()
        || loop_state.task_checkpoint.is_some()
        || loop_state.pending_user_input_required
    {
        return Ok(None);
    }
    let batch_len = super::action_batch_contract::independent_read_batch_prefix_len(
        state,
        actions,
        policy.max_actions_per_turn.max(1),
    );
    if batch_len == 0 {
        return Ok(None);
    }
    if loop_state.task_budget_slice.as_ref().is_some_and(|slice| {
        (loop_state.tool_calls_total as u64).saturating_add(batch_len as u64)
            > slice.hard_ceilings.tool_calls
    }) {
        return Ok(None);
    }
    if actions[..batch_len].iter().any(|action| {
        prior_structured_observation_satisfies_read_only_action(
            state,
            agent_run_context,
            loop_state,
            action,
        )
    }) {
        return Ok(None);
    }

    let fingerprints = actions[..batch_len]
        .iter()
        .map(|action| super::action_fingerprint_for_policy(state, policy, action))
        .collect::<Vec<_>>();
    for (idx, (action, fingerprint)) in actions[..batch_len].iter().zip(&fingerprints).enumerate() {
        if let Some(reason) = check_repeat_action_guard(
            state,
            task,
            loop_state,
            policy,
            action,
            fingerprint,
            idx + 1,
        ) {
            return Ok(Some(finalize_execute_round_outcome(
                loop_state,
                snapshot,
                actionable_count,
                0,
                false,
                Some(reason),
            )));
        }
        info!(
            "executor_parallel_read_start task_id={} round={} step={} action={}",
            task.task_id,
            loop_state.round_no,
            idx + 1,
            plan_step_label(action)
        );
    }

    let batch = super::parallel_read_batch::dispatch_independent_read_batch(
        state,
        task,
        goal,
        user_text,
        actions,
        round_steps,
        loop_state,
        policy,
        &fingerprints,
        batch_len,
        agent_run_context,
    )
    .await?;
    crate::task_event_transport::publish_loop_state_snapshot(state, task, user_text, loop_state);
    info!(
        "executor_parallel_read_complete task_id={} round={} batch_size={} executed={} stop_signal={}",
        task.task_id,
        loop_state.round_no,
        batch_len,
        batch.executed_actions,
        batch.stop_signal
    );
    Ok(Some(finalize_execute_round_outcome(
        loop_state,
        snapshot,
        actionable_count,
        batch.executed_actions,
        batch.ended_with_user_visible_output,
        Some(batch.stop_signal),
    )))
}

pub(super) async fn execute_actions_once(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<RoundOutcome, String> {
    ensure_task_running(state, task)?;
    let mut executed_actions = 0usize;
    let mut stop_signal: Option<String> = None;
    let actionable_count = actions
        .iter()
        .take(policy.max_actions_per_turn.max(1))
        .count();
    let snapshot = capture_round_progress_snapshot(loop_state);
    let mut ended_with_user_visible_output = false;
    let round_steps: Vec<String> = actions.iter().map(plan_step_label).collect();
    if let Some(outcome) = try_execute_independent_read_batch(
        state,
        task,
        goal,
        user_text,
        actions,
        &round_steps,
        loop_state,
        policy,
        agent_run_context,
        &snapshot,
        actionable_count,
    )
    .await?
    {
        return Ok(outcome);
    }
    for (idx, action) in actions
        .iter()
        .take(policy.max_actions_per_turn.max(1))
        .enumerate()
    {
        ensure_task_running(state, task)?;
        let step_in_round = idx + 1;
        let global_step = loop_state.total_steps_executed + 1;
        let fingerprint = super::action_fingerprint_for_policy(state, policy, action);
        if action_counts_as_tool_call(action)
            && loop_state.task_budget_slice.as_ref().is_some_and(|slice| {
                loop_state.tool_calls_total as u64 >= slice.hard_ceilings.tool_calls
            })
        {
            info!(
                "executor_result_error task_id={} round={} step={} type=guard error=task_budget_admin_tool_ceiling reached={} action={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                loop_state.tool_calls_total,
                plan_step_label(action)
            );
            stop_signal = Some("task_budget_admin_tool_ceiling".to_string());
            break;
        }
        if prior_structured_observation_satisfies_read_only_action(
            state,
            agent_run_context,
            loop_state,
            action,
        ) {
            info!(
                "executor_structured_observation_skip_redundant_read task_id={} round={} step={} action={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                plan_step_label(action)
            );
            stop_signal = Some("structured_observation_already_ready".to_string());
            break;
        }
        if let Some(reason) = check_repeat_action_guard(
            state,
            task,
            loop_state,
            policy,
            action,
            &fingerprint,
            step_in_round,
        ) {
            stop_signal = Some(reason);
            break;
        }

        info!(
            "executor_step_start task_id={} round={} step={} global_step={} action={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            global_step,
            plan_step_label(action)
        );
        loop_state.last_actions_fingerprint = Some(fingerprint.clone());
        let decision = dispatch_round_action(
            state,
            task,
            goal,
            user_text,
            actions,
            &round_steps,
            loop_state,
            policy,
            idx,
            action,
            &fingerprint,
            global_step,
            step_in_round,
            &mut executed_actions,
            &mut ended_with_user_visible_output,
            agent_run_context,
        )
        .await?;
        crate::task_event_transport::publish_loop_state_snapshot(
            state, task, user_text, loop_state,
        );
        let executed_limit = policy.max_actions_per_turn.max(1);
        let remaining_actions = &actions[idx + 1..actions.len().min(executed_limit)];
        if matches!(
            decision,
            ActionLoopDecision::NextAction | ActionLoopDecision::ContinueRound
        ) {
            if let Some(reason_code) =
                super::action_batch_contract::return_control_boundary_after_action(
                    state,
                    actions,
                    idx,
                    executed_limit,
                )
            {
                info!(
                    "executor_action_batch_boundary task_id={} round={} step={} reason_code={} remaining={}",
                    task.task_id,
                    loop_state.round_no,
                    step_in_round,
                    reason_code,
                    remaining_actions.len()
                );
                stop_signal = Some(reason_code.to_string());
                break;
            }
        }
        if matches!(
            decision,
            ActionLoopDecision::NextAction | ActionLoopDecision::ContinueRound
        ) && successful_structured_observation_satisfies_selector(
            agent_run_context,
            loop_state,
            action,
            remaining_actions,
        ) {
            info!(
                "executor_structured_observation_skip_terminal_discussion task_id={} round={} step={} remaining={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                remaining_actions.len()
            );
            stop_signal = Some("structured_observation_ready".to_string());
            break;
        }
        if matches!(
            decision,
            ActionLoopDecision::NextAction | ActionLoopDecision::ContinueRound
        ) && terminal_synthesis_can_skip_remaining_actions(action, remaining_actions, loop_state)
        {
            info!(
                "executor_terminal_synthesis_skip_placeholder_delivery task_id={} round={} step={} remaining={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                remaining_actions.len()
            );
            stop_signal = Some("terminal_synthesis_ready".to_string());
            break;
        }
        match decision {
            ActionLoopDecision::NextAction => {}
            ActionLoopDecision::ContinueRound => continue,
            ActionLoopDecision::StopRound(reason) => {
                stop_signal = Some(reason);
                break;
            }
        }
    }
    Ok(finalize_execute_round_outcome(
        loop_state,
        &snapshot,
        actionable_count,
        executed_actions,
        ended_with_user_visible_output,
        stop_signal,
    ))
}

#[cfg(test)]
#[path = "execution_loop_tests.rs"]
mod tests;

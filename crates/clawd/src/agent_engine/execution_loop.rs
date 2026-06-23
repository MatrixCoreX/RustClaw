use tracing::info;

use super::{
    dispatch_round_action, ensure_task_running, plan_step_label, ActionLoopDecision,
    AgentLoopGuardPolicy, AgentRunContext, LoopState, RoundOutcome,
};
use crate::{AgentAction, AppState, ClaimedTask};

struct RoundProgressSnapshot {
    delivery_count: usize,
    progress_count: usize,
    subtask_count: usize,
}

fn capture_round_progress_snapshot(loop_state: &LoopState) -> RoundProgressSnapshot {
    RoundProgressSnapshot {
        delivery_count: loop_state.delivery_messages.len(),
        progress_count: loop_state.progress_messages.len(),
        subtask_count: loop_state.subtask_results.len(),
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
    let progress_grew = loop_state.progress_messages.len() > snapshot.progress_count;
    let step_output_grew = loop_state.subtask_results.len() > snapshot.subtask_count;
    let no_progress = !delivery_grew && !progress_grew && !step_output_grew;
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
}

fn repeated_successful_observe_or_validate_is_allowed(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
) -> bool {
    let Some(effect) = action_effect_for_repeat_guard(state, loop_state, action) else {
        return false;
    };
    !effect.mutates && (effect.observes || effect.validates)
}

fn action_effect_for_repeat_guard(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
) -> Option<crate::execution_recipe::ActionEffect> {
    let (skill_name, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        AgentAction::CallCapability { .. } => return None,
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

fn check_repeat_action_guard(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    action: &AgentAction,
    route_result: Option<&crate::RouteResult>,
    fingerprint: &str,
    step_in_round: usize,
) -> Option<String> {
    if matches!(action, AgentAction::Respond { .. }) {
        return None;
    }
    let repeat_count = loop_state
        .repeat_action_counts
        .entry(fingerprint.to_string())
        .or_insert(0);
    *repeat_count += 1;
    if *repeat_count > policy.repeat_action_limit {
        if let Some(attribution) = super::registry_idempotency_guard_attribution(
            state,
            policy,
            action,
            route_result,
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
        if repeated_successful_action_is_allowed_for_active_recipe(state, loop_state, action)
            || repeated_successful_observe_or_validate_is_allowed(state, loop_state, action)
        {
            return None;
        }
        if let Some(attribution) = super::registry_idempotency_guard_attribution(
            state,
            policy,
            action,
            route_result,
            fingerprint,
            "registry_idempotency_repeat_completed_action",
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
        return Some("repeat_completed_action".to_string());
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
    let actionable_count = actions.iter().take(policy.max_steps.max(1)).count();
    let snapshot = capture_round_progress_snapshot(loop_state);
    let mut ended_with_user_visible_output = false;
    let round_steps: Vec<String> = actions.iter().map(plan_step_label).collect();
    let route_result = agent_run_context.and_then(|ctx| ctx.route_result.as_ref());
    for (idx, action) in actions.iter().take(policy.max_steps.max(1)).enumerate() {
        ensure_task_running(state, task)?;
        let step_in_round = idx + 1;
        let global_step = loop_state.total_steps_executed + 1;
        let fingerprint = super::action_fingerprint_for_policy(state, policy, action, route_result);
        if action_counts_as_tool_call(action)
            && loop_state.tool_calls_total >= policy.max_tool_calls.max(1)
        {
            info!(
                "executor_result_error task_id={} round={} step={} type=guard error=max_tool_calls reached={} action={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                policy.max_tool_calls,
                plan_step_label(action)
            );
            stop_signal = Some("max_tool_calls".to_string());
            break;
        }
        if let Some(reason) = check_repeat_action_guard(
            state,
            task,
            loop_state,
            policy,
            action,
            route_result,
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
        match dispatch_round_action(
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
        .await?
        {
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

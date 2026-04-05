use tracing::info;

use super::{
    dispatch_round_action, ensure_task_running, plan_step_label, ActionLoopDecision,
    AgentLoopGuardPolicy, LoopState, RoundOutcome,
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

fn check_repeat_action_guard(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    fingerprint: &str,
    step_in_round: usize,
) -> Option<String> {
    let repeat_count = loop_state
        .repeat_action_counts
        .entry(fingerprint.to_string())
        .or_insert(0);
    *repeat_count += 1;
    if let Some(success_count) = loop_state.successful_action_fingerprints.get(fingerprint) {
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
    if *repeat_count > policy.repeat_action_limit {
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
    None
}

pub(super) async fn execute_actions_once(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
) -> Result<RoundOutcome, String> {
    ensure_task_running(state, task)?;
    let mut executed_actions = 0usize;
    let mut stop_signal: Option<String> = None;
    let actionable_count = actions.iter().take(policy.max_steps.max(1)).count();
    let snapshot = capture_round_progress_snapshot(loop_state);
    let mut ended_with_user_visible_output = false;
    let round_steps: Vec<String> = actions.iter().map(plan_step_label).collect();
    for (idx, action) in actions.iter().take(policy.max_steps.max(1)).enumerate() {
        ensure_task_running(state, task)?;
        let step_in_round = idx + 1;
        let global_step = loop_state.total_steps_executed + 1;
        let fingerprint = super::action_fingerprint(state, action);
        if let Some(reason) =
            check_repeat_action_guard(task, loop_state, policy, &fingerprint, step_in_round)
        {
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
mod tests {
    use super::{capture_round_progress_snapshot, finalize_execute_round_outcome};

    #[test]
    fn observed_output_alone_does_not_mark_plan_exhausted_user_visible() {
        let loop_state = super::LoopState::new(2);
        let snapshot = capture_round_progress_snapshot(&loop_state);
        let outcome = finalize_execute_round_outcome(&loop_state, &snapshot, 1, 1, false, None);
        assert!(outcome.stop_signal.is_none());
    }

    #[test]
    fn explicit_user_visible_output_marks_plan_exhausted() {
        let loop_state = super::LoopState::new(2);
        let snapshot = capture_round_progress_snapshot(&loop_state);
        let outcome = finalize_execute_round_outcome(&loop_state, &snapshot, 1, 1, true, None);
        assert_eq!(
            outcome.stop_signal.as_deref(),
            Some("plan_exhausted_user_visible")
        );
    }
}

use std::collections::HashMap;

use futures_util::future::join_all;

use super::{
    dispatch_round_action, ActionLoopDecision, AgentLoopGuardPolicy, AgentRunContext, LoopState,
};
use crate::{AgentAction, AppState, ClaimedTask};

pub(super) struct ParallelReadBatchResult {
    pub(super) executed_actions: usize,
    pub(super) ended_with_user_visible_output: bool,
    pub(super) stop_signal: String,
}

struct ChildReadResult {
    state: LoopState,
    executed_actions: usize,
    ended_with_user_visible_output: bool,
    decision: Result<ActionLoopDecision, String>,
}

fn append_delta<T: Clone>(target: &mut Vec<T>, baseline: &[T], child: &[T]) {
    target.extend(child.iter().skip(baseline.len()).cloned());
}

fn merge_count_deltas(
    target: &mut HashMap<String, usize>,
    baseline: &HashMap<String, usize>,
    child: &HashMap<String, usize>,
) {
    for (key, child_count) in child {
        let delta = child_count.saturating_sub(baseline.get(key).copied().unwrap_or_default());
        if delta > 0 {
            *target.entry(key.clone()).or_insert(0) += delta;
        }
    }
}

fn merge_child_read_state(target: &mut LoopState, baseline: &LoopState, child: &LoopState) {
    target.tool_calls_total = target.tool_calls_total.saturating_add(
        child
            .tool_calls_total
            .saturating_sub(baseline.tool_calls_total),
    );
    target.total_steps_executed = target.total_steps_executed.saturating_add(
        child
            .total_steps_executed
            .saturating_sub(baseline.total_steps_executed),
    );
    target
        .loaded_capability_skills
        .extend(child.loaded_capability_skills.iter().cloned());
    target
        .loaded_mcp_capabilities
        .extend(child.loaded_mcp_capabilities.iter().cloned());
    for scope in &child.active_capability_scopes {
        if !target.active_capability_scopes.contains(scope) {
            target.active_capability_scopes.push(scope.clone());
        }
    }
    append_delta(
        &mut target.progress_messages,
        &baseline.progress_messages,
        &child.progress_messages,
    );
    append_delta(
        &mut target.delivery_messages,
        &baseline.delivery_messages,
        &child.delivery_messages,
    );
    append_delta(
        &mut target.subtask_results,
        &baseline.subtask_results,
        &child.subtask_results,
    );
    append_delta(
        &mut target.history_compact,
        &baseline.history_compact,
        &child.history_compact,
    );
    append_delta(
        &mut target.attempt_ledger_entries,
        &baseline.attempt_ledger_entries,
        &child.attempt_ledger_entries,
    );
    append_delta(
        &mut target.capability_results,
        &baseline.capability_results,
        &child.capability_results,
    );
    append_delta(
        &mut target.executed_step_results,
        &baseline.executed_step_results,
        &child.executed_step_results,
    );
    append_delta(
        &mut target.round_traces,
        &baseline.round_traces,
        &child.round_traces,
    );
    append_delta(
        &mut target.rollout_attribution,
        &baseline.rollout_attribution,
        &child.rollout_attribution,
    );
    append_delta(
        &mut target.task_observations,
        &baseline.task_observations,
        &child.task_observations,
    );
    merge_count_deltas(
        &mut target.successful_action_fingerprints,
        &baseline.successful_action_fingerprints,
        &child.successful_action_fingerprints,
    );
    for (key, value) in &child.output_vars {
        if baseline.output_vars.get(key) != Some(value) {
            target.output_vars.insert(key.clone(), value.clone());
        }
    }
    for (key, value) in &child.written_file_aliases {
        if baseline.written_file_aliases.get(key) != Some(value) {
            target
                .written_file_aliases
                .insert(key.clone(), value.clone());
        }
    }

    if child.last_output != baseline.last_output {
        target.last_output = child.last_output.clone();
    }
    if child.last_written_file_path != baseline.last_written_file_path {
        target.last_written_file_path = child.last_written_file_path.clone();
    }
    if child.last_user_visible_respond != baseline.last_user_visible_respond {
        target.last_user_visible_respond = child.last_user_visible_respond.clone();
    }
    if child.last_publishable_synthesis_output != baseline.last_publishable_synthesis_output {
        target.last_publishable_synthesis_output = child.last_publishable_synthesis_output.clone();
    }
    if child.last_capability_synthesis_output != baseline.last_capability_synthesis_output {
        target.last_capability_synthesis_output = child.last_capability_synthesis_output.clone();
    }
    if child.latest_validation_result != baseline.latest_validation_result {
        target.latest_validation_result = child.latest_validation_result.clone();
    }
    if child.task_lifecycle != baseline.task_lifecycle {
        target.task_lifecycle = child.task_lifecycle.clone();
    }
    if child.task_checkpoint != baseline.task_checkpoint {
        target.task_checkpoint = child.task_checkpoint.clone();
    }
    if child.execution_recipe != baseline.execution_recipe {
        target.execution_recipe = child.execution_recipe;
    }
    target.has_tool_or_skill_output |= child.has_tool_or_skill_output;
    target.has_recoverable_failure_context |= child.has_recoverable_failure_context;
    target.pending_user_input_required |= child.pending_user_input_required;
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_independent_read_batch(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    fingerprints: &[String],
    batch_len: usize,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<ParallelReadBatchResult, String> {
    let baseline = loop_state.clone();
    let futures = (0..batch_len).map(|idx| {
        let mut child_state = baseline.clone();
        let action = actions[idx].clone();
        let fingerprint = fingerprints[idx].clone();
        async move {
            let mut executed_actions = 0;
            let mut ended_with_user_visible_output = false;
            let decision = dispatch_round_action(
                state,
                task,
                goal,
                user_text,
                actions,
                round_steps,
                &mut child_state,
                policy,
                idx,
                &action,
                &fingerprint,
                baseline.total_steps_executed + idx + 1,
                idx + 1,
                &mut executed_actions,
                &mut ended_with_user_visible_output,
                agent_run_context,
            )
            .await;
            ChildReadResult {
                state: child_state,
                executed_actions,
                ended_with_user_visible_output,
                decision,
            }
        }
    });
    let children = join_all(futures).await;
    let mut executed_actions = 0;
    let mut ended_with_user_visible_output = false;
    let mut stop_signal = "independent_read_batch_observed".to_string();
    let mut first_error = None;
    for child in children {
        merge_child_read_state(loop_state, &baseline, &child.state);
        executed_actions += child.executed_actions;
        ended_with_user_visible_output |= child.ended_with_user_visible_output;
        match child.decision {
            Ok(ActionLoopDecision::StopRound(reason)) => stop_signal = reason,
            Ok(ActionLoopDecision::NextAction | ActionLoopDecision::ContinueRound) => {}
            Err(error) if first_error.is_none() => first_error = Some(error),
            Err(_) => {}
        }
    }
    loop_state.last_actions_fingerprint = fingerprints.get(batch_len - 1).cloned();
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(ParallelReadBatchResult {
        executed_actions,
        ended_with_user_visible_output,
        stop_signal,
    })
}

#[cfg(test)]
#[path = "parallel_read_batch_tests.rs"]
mod tests;

use tracing::info;

use super::{
    dispatch_round_action, ensure_task_running, plan_step_label, ActionLoopDecision,
    AgentLoopGuardPolicy, AgentRunContext, LoopState, RoundOutcome,
};
use crate::{AgentAction, AppState, ClaimedTask};
use serde_json::{json, Value};
use std::collections::BTreeSet;

struct RoundProgressSnapshot {
    delivery_count: usize,
    machine_progress_fingerprints: BTreeSet<String>,
}

fn active_tool_event_payload(
    action: &AgentAction,
    round_no: usize,
    step_in_round: usize,
    global_step: usize,
) -> Option<Value> {
    let command_preview = command_preview_for_action(action);
    let (action_kind, action_ref, tool_or_skill, requested_capability) = match action {
        AgentAction::CallTool { tool, .. } => {
            ("call_tool", tool.as_str(), Some(tool.as_str()), None)
        }
        AgentAction::CallSkill { skill, .. } => {
            ("call_skill", skill.as_str(), Some(skill.as_str()), None)
        }
        AgentAction::CallCapability { capability, .. } => (
            "call_capability",
            capability.as_str(),
            None,
            Some(capability.as_str()),
        ),
        AgentAction::Think { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. } => return None,
    };
    Some(json!({
        "schema_version": 1,
        "phase": "active",
        "round_no": round_no,
        "step_in_round": step_in_round,
        "global_step": global_step,
        "action_kind": action_kind,
        "action_ref": action_ref,
        "tool_or_skill": tool_or_skill,
        "requested_capability": requested_capability,
        "command_preview": command_preview,
        "status": "running",
    }))
}

fn command_preview_for_action(action: &AgentAction) -> Option<String> {
    let args = match action {
        AgentAction::CallTool { args, .. }
        | AgentAction::CallSkill { args, .. }
        | AgentAction::CallCapability { args, .. } => args,
        AgentAction::Think { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. } => return None,
    };
    let command = args
        .get("command")
        .or_else(|| args.get("cmd"))
        .and_then(Value::as_str)?;
    safe_command_preview(command)
}

fn safe_command_preview(command: &str) -> Option<String> {
    let first_line = command.lines().next()?.trim();
    let mut tokens = first_line.split_whitespace();
    let mut executable = tokens.next()?;
    while executable.contains('=') && !executable.starts_with('=') {
        executable = tokens.next()?;
    }
    let executable = executable.trim_matches(['\'', '"']);
    let executable = executable.rsplit('/').next().unwrap_or(executable);
    if executable.is_empty()
        || !executable
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return None;
    }
    let mut preview = executable.chars().take(32).collect::<String>();
    if matches!(executable, "curl" | "wget" | "bash" | "sh" | "zsh" | "fish") {
        return Some(preview);
    }
    if let Some(subcommand) = tokens.next() {
        let subcommand = subcommand.trim_matches(['\'', '"']);
        if !subcommand.starts_with('-')
            && subcommand.len() <= 32
            && subcommand
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        {
            preview.push(' ');
            preview.push_str(subcommand);
        }
    }
    Some(preview)
}

fn publish_active_tool_event(
    state: &AppState,
    task: &ClaimedTask,
    action: &AgentAction,
    round_no: usize,
    step_in_round: usize,
    global_step: usize,
) {
    let Some(payload) = active_tool_event_payload(action, round_no, step_in_round, global_step)
    else {
        return;
    };
    if let Err(error) =
        crate::task_event_transport::publish_claimed_event(state, task, "tool_active", payload)
    {
        info!(
            "executor_tool_active_event_error task_id={} round={} step={} error={}",
            task.task_id,
            round_no,
            step_in_round,
            crate::truncate_for_log(&error.to_string())
        );
    }
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
        || registry_allows_repeated_idempotent_action(state, action)
        || completed_terminal_termination_allows_replay(state, loop_state, action)
        || terminal_poll_after_termination_is_fresh(state, loop_state, action)
}

fn terminal_poll_after_termination_is_fresh(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
) -> bool {
    let (skill_name, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        AgentAction::CallCapability { .. } => {
            let resolved =
                crate::capability_resolver::resolve_agent_action_for_state(state, action.clone());
            if matches!(resolved, AgentAction::CallCapability { .. }) {
                return false;
            }
            return terminal_poll_after_termination_is_fresh(state, loop_state, &resolved);
        }
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => return false,
    };
    if state.resolve_canonical_skill_name(skill_name) != "run_cmd"
        || args.get("action").and_then(Value::as_str) != Some("terminal_poll")
    {
        return false;
    }
    let Some(target_session_id) = args
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
    else {
        return false;
    };

    let successful_outputs = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .filter(|(_, step)| step.is_ok() && step.skill == "run_cmd")
        .filter_map(|(index, step)| {
            step.output
                .as_deref()
                .and_then(|output| serde_json::from_str::<Value>(output).ok())
                .map(|output| (index, output))
        })
        .collect::<Vec<_>>();
    let mut session_ids = successful_outputs
        .iter()
        .filter_map(|(_, output)| output.get("session_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
        .collect::<Vec<_>>();
    session_ids.sort_unstable();
    session_ids.dedup();
    if session_ids.as_slice() != [target_session_id] {
        return false;
    }

    let last_poll = successful_outputs.iter().rev().find_map(|(index, output)| {
        (output.get("session_id").and_then(Value::as_str) == Some(target_session_id)
            && output.get("page").is_some())
        .then_some(*index)
    });
    let last_termination = successful_outputs.iter().rev().find_map(|(index, output)| {
        (output.get("action").and_then(Value::as_str) == Some("terminal_terminate")
            && output.get("status").and_then(Value::as_str) == Some("ok"))
        .then_some(*index)
    });
    matches!(
        (last_poll, last_termination),
        (Some(poll), Some(termination)) if termination > poll
    )
}

fn completed_terminal_termination_allows_replay(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
) -> bool {
    let (skill_name, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        AgentAction::CallCapability { .. } => {
            let resolved =
                crate::capability_resolver::resolve_agent_action_for_state(state, action.clone());
            if matches!(resolved, AgentAction::CallCapability { .. }) {
                return false;
            }
            return completed_terminal_termination_allows_replay(state, loop_state, &resolved);
        }
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => return false,
    };
    if state.resolve_canonical_skill_name(skill_name) != "run_cmd"
        || args.get("action").and_then(Value::as_str) != Some("terminal_terminate")
    {
        return false;
    }

    let successful_outputs = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && step.skill == "run_cmd")
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<Value>(output).ok())
        .collect::<Vec<_>>();
    let observed_termination_count = successful_outputs
        .iter()
        .filter(|output| {
            output.get("action").and_then(Value::as_str) == Some("terminal_terminate")
                && output.get("status").and_then(Value::as_str) == Some("ok")
                && output
                    .pointer("/data/termination_requested")
                    .and_then(Value::as_bool)
                    .is_some()
        })
        .count();
    let mut session_ids = successful_outputs
        .iter()
        .filter_map(|output| output.get("session_id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
        .collect::<Vec<_>>();
    session_ids.sort_unstable();
    session_ids.dedup();
    observed_termination_count == 1 && session_ids.len() == 1
}

fn registry_allows_repeated_idempotent_action(state: &AppState, action: &AgentAction) -> bool {
    let (skill_name, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        AgentAction::CallCapability { .. } => {
            let resolved =
                crate::capability_resolver::resolve_agent_action_for_state(state, action.clone());
            if matches!(resolved, AgentAction::CallCapability { .. }) {
                return false;
            }
            return registry_allows_repeated_idempotent_action(state, &resolved);
        }
        AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => return false,
    };
    let canonical_skill = state.resolve_canonical_skill_name(skill_name);
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    state.get_skills_registry().is_some_and(|registry| {
        registry.resolved_idempotent(&canonical_skill, action)
            && !registry.resolved_once_per_task(&canonical_skill, action)
            && crate::execution_recipe::classify_skill_action_effect(state, &canonical_skill, args)
                .mutates
    })
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
    let repeated_action_allowed =
        repeated_successful_action_is_allowed_for_active_recipe(state, loop_state, action);
    let repeat_count = loop_state
        .repeat_action_counts
        .entry(fingerprint.to_string())
        .or_insert(0);
    *repeat_count += 1;
    if *repeat_count > policy.repeat_action_limit && !repeated_action_allowed {
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
    if let Some(failure_count) = loop_state.failed_action_fingerprints.get(fingerprint) {
        if !repeated_action_allowed {
            if let Some(attribution) = super::registry_idempotency_guard_attribution(
                state,
                policy,
                action,
                fingerprint,
                "registry_idempotency_repeat_failed_action",
                Some(*failure_count),
                None,
            ) {
                loop_state.rollout_attribution.push(attribution);
            }
            loop_state.output_vars.insert(
                "agent_loop.repeat_failed_action".to_string(),
                json!({
                    "status_code": "repeat_failed_action_blocked",
                    "failure_count": failure_count,
                    "forbidden_action_fingerprint": fingerprint,
                    "recovery_options": [
                        "change_action_or_arguments",
                        "respond_from_structured_failure"
                    ]
                })
                .to_string(),
            );
            loop_state.history_compact.push(format!(
                "round={} step={} repeat_failed_action_blocked fingerprint={}",
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_agent_trace(fingerprint)
            ));
            info!(
                "executor_result_error task_id={} round={} step={} type=guard error=repeat_failed_action_blocked action={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_log(fingerprint)
            );
            return Some("repeat_failed_action".to_string());
        }
    }
    if let Some(success_count) = loop_state.successful_action_fingerprints.get(fingerprint) {
        if repeated_action_allowed {
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
        if let Some(previous) = loop_state.capability_results.iter().rev().find(|result| {
            result.status == claw_core::capability_result::CapabilityResultStatus::Ok
                && result
                    .provenance
                    .get("action_fingerprint")
                    .and_then(Value::as_str)
                    == Some(fingerprint)
        }) {
            let identity = previous.canonical_evidence_identity();
            let reused = json!({
                "schema_version": 1,
                "observation_kind": "completed_action_result_reused",
                "result_ref": identity.evidence_id,
                "result_sha256": identity.sha256,
                "capability": previous.capability,
                "artifact_refs": previous.artifacts.iter().filter_map(|artifact| artifact.artifact_ref.as_deref()).collect::<Vec<_>>(),
                "data": previous.data,
            });
            let serialized = reused.to_string();
            loop_state.last_output = Some(serialized.clone());
            loop_state
                .output_vars
                .insert("agent_loop.repeat_completed_result".to_string(), serialized);
            loop_state.task_observations.push(reused);
            loop_state.history_compact.push(format!(
                "round={} step={} completed_action_result_reused result_ref={}",
                loop_state.round_no, step_in_round, identity.evidence_id
            ));
            return Some("completed_action_result_reused".to_string());
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
            crate::machine_selector::structured_json_satisfies_field_selector(selector, output)
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
            if reason == "completed_action_result_reused" {
                return Ok(None);
            }
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

fn action_observation_boundary(
    planned_action_count: usize,
    executed_action_count: usize,
    observation_boundary: usize,
) -> Option<Value> {
    (planned_action_count > observation_boundary
        && executed_action_count >= observation_boundary)
        .then(|| {
            serde_json::json!({
                "owner_layer": "execution_scheduler",
                "state": "continue",
                "reason_code": "action_observation_boundary",
                "complete": false,
                "planned_action_count": planned_action_count,
                "executed_action_count": executed_action_count,
                "remaining_action_count": planned_action_count.saturating_sub(executed_action_count),
                "recovery_action": "replan_from_latest_observation",
            })
    })
}

fn record_deferred_plan_tail(
    loop_state: &mut LoopState,
    remaining_actions: &[AgentAction],
    boundary_reason: &str,
) {
    let action_refs = remaining_actions
        .iter()
        .filter_map(|action| match action {
            AgentAction::CallCapability { capability, .. } => {
                Some(format!("capability:{capability}"))
            }
            AgentAction::CallTool { tool, .. } => Some(format!("tool:{tool}")),
            AgentAction::CallSkill { skill, .. } => Some(format!("skill:{skill}")),
            AgentAction::SynthesizeAnswer { .. } => Some("synthesize_answer".to_string()),
            AgentAction::Respond { .. } => Some("respond".to_string()),
            AgentAction::Think { .. } => None,
        })
        .collect::<Vec<_>>();
    if action_refs.is_empty() {
        return;
    }
    loop_state.task_observations.push(serde_json::json!({
        "observation_kind": "deferred_plan_tail",
        "owner_layer": "execution_scheduler",
        "state": "continue",
        "complete": false,
        "boundary_reason": boundary_reason,
        "remaining_action_refs": action_refs,
        "recovery_action": "replan_unfinished_actions_from_latest_observation",
    }));
    loop_state.history_compact.push(format!(
        "round={} deferred_plan_tail boundary={} remaining_action_refs={}",
        loop_state.round_no,
        boundary_reason,
        action_refs.join(",")
    ));
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
    if super::loop_control::active_task_boundary_control_pending(state, task)? {
        record_deferred_plan_tail(loop_state, actions, "active_task_control_boundary");
        return Ok(RoundOutcome {
            executed_actions: 0,
            had_error: false,
            stop_signal: Some("active_task_control_boundary".to_string()),
            next_goal_hint: None,
            no_progress: false,
        });
    }
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
        if super::loop_control::active_task_boundary_control_pending(state, task)? {
            stop_signal = Some("active_task_control_boundary".to_string());
            record_deferred_plan_tail(
                loop_state,
                &actions[idx..actions.len().min(policy.max_actions_per_turn.max(1))],
                "active_task_control_boundary",
            );
            break;
        }
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
            if reason == "completed_action_result_reused" {
                executed_actions += 1;
                continue;
            }
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
        publish_active_tool_event(
            state,
            task,
            action,
            loop_state.round_no,
            step_in_round,
            global_step,
        );
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
        if super::loop_control::active_task_boundary_control_pending(state, task)? {
            record_deferred_plan_tail(
                loop_state,
                remaining_actions,
                "active_task_control_boundary",
            );
            stop_signal = Some("active_task_control_boundary".to_string());
            break;
        }
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
                record_deferred_plan_tail(loop_state, remaining_actions, reason_code);
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
    let observation_boundary = policy.max_actions_per_turn.max(1);
    if let Some(observation) = stop_signal
        .is_none()
        .then(|| action_observation_boundary(actions.len(), executed_actions, observation_boundary))
        .flatten()
    {
        loop_state.task_observations.push(observation);
        stop_signal = Some("action_observation_boundary".to_string());
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

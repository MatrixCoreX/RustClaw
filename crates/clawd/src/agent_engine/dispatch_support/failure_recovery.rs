use serde_json::Value;
use tracing::info;

use super::{
    append_progress_hint, encode_progress_i18n, AgentLoopGuardPolicy, AppState, ClaimedTask,
    LoopState,
};
use crate::agent_engine::{
    attempt_ledger, CLAWD_CONTINUE_ON_ERROR_ARG, CLAWD_LITERAL_COMMAND_ARG,
    CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG,
};
use crate::AgentAction;

fn is_discussion_only_action(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::Respond { .. }
            | AgentAction::SynthesizeAnswer { .. }
            | AgentAction::Think { .. }
    )
}

pub(crate) fn active_recipe_terminal_discussion_should_replan(
    actions: &[AgentAction],
    loop_state: &LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
) -> bool {
    if !loop_state.execution_recipe.is_active()
        || matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Done
        )
    {
        return false;
    }
    if !loop_state.executed_step_results.iter().any(|step| {
        !matches!(
            step.skill.as_str(),
            "respond" | "synthesize_answer" | "think"
        )
    }) {
        return false;
    }
    !actions
        .iter()
        .take(policy.max_actions_per_turn.max(1))
        .skip(idx + 1)
        .any(|action| !is_discussion_only_action(action))
}

pub(super) fn record_active_recipe_terminal_discussion_replan(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    action_kind: &str,
) {
    let reason = "active_recipe_terminal_discussion_before_done";
    loop_state.has_recoverable_failure_context = true;
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        action_kind,
        false,
        reason,
    );
    attempt_ledger::record_attempt_with_retry_instruction(
        loop_state,
        action_kind,
        reason,
        crate::executor::StepExecutionStatus::Error,
        reason,
        Some("active_recipe_incomplete_terminal_discussion"),
        reason,
        Some("active_recipe_continue_required"),
    );
    append_progress_hint(
        state,
        task,
        &mut loop_state.progress_messages,
        encode_progress_i18n("telegram.progress.retry_replan", &[]),
    );
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: format!("step_{global_step}"),
            skill: action_kind.to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some(reason.to_string()),
            started_at: 0,
            finished_at: 0,
        });
    loop_state.history_compact.push(format!(
        "round={} step={} {} active_recipe_terminal_discussion_before_done phase={}",
        loop_state.round_no,
        step_in_round,
        action_kind,
        loop_state.execution_recipe.phase.as_str()
    ));
    info!(
        "active_recipe_terminal_discussion_replan task_id={} round={} step={} action={} phase={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_kind,
        loop_state.execution_recipe.phase.as_str()
    );
}

pub(super) fn has_remaining_action_after(
    actions: &[AgentAction],
    current_idx: usize,
    max_actions_per_turn: usize,
) -> bool {
    actions
        .iter()
        .take(max_actions_per_turn.max(1))
        .skip(current_idx + 1)
        .any(|action| !matches!(action, AgentAction::Think { .. }))
}

fn has_remaining_action_after_full(actions: &[AgentAction], current_idx: usize) -> bool {
    actions
        .iter()
        .skip(current_idx + 1)
        .any(|action| !matches!(action, AgentAction::Think { .. }))
}

fn remaining_actions_are_discussion_only(
    actions: &[AgentAction],
    current_idx: usize,
    max_actions_per_turn: usize,
) -> bool {
    let remaining = actions
        .iter()
        .take(max_actions_per_turn.max(1))
        .skip(current_idx + 1)
        .filter(|action| !matches!(action, AgentAction::Think { .. }))
        .collect::<Vec<_>>();
    !remaining.is_empty()
        && remaining.iter().all(|action| match action {
            AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. } => true,
            _ => false,
        })
}

fn remaining_actions_after_plan_capacity_are_discussion_only(
    actions: &[AgentAction],
    current_idx: usize,
    max_actions_per_turn: usize,
) -> bool {
    if current_idx + 1 < max_actions_per_turn.max(1) {
        return false;
    }
    let remaining = actions
        .iter()
        .skip(current_idx + 1)
        .filter(|action| !matches!(action, AgentAction::Think { .. }))
        .collect::<Vec<_>>();
    !remaining.is_empty()
        && remaining.iter().all(|action| match action {
            AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. } => true,
            _ => false,
        })
}

fn run_cmd_should_continue_after_split_failure(args: Option<&Value>) -> bool {
    args.and_then(|value| value.get(CLAWD_CONTINUE_ON_ERROR_ARG))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn run_cmd_is_literal_user_command(args: Option<&Value>) -> bool {
    args.and_then(|value| value.get(CLAWD_LITERAL_COMMAND_ARG))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn run_cmd_literal_failure_is_repairable(args: Option<&Value>) -> bool {
    args.and_then(|value| value.get(CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn structured_error_kind(err: &str) -> Option<String> {
    crate::skills::parse_structured_skill_error(err).map(|structured| structured.error_kind)
}

fn planner_can_repair_structured_skill_error(err: &str) -> bool {
    structured_error_kind(err).is_some_and(|kind| {
        matches!(
            kind.as_str(),
            "unsupported_action"
                | "invalid_input"
                | "invalid_args"
                | "schema_error"
                | "missing_required_field"
                | "timeout"
                | "idle_timeout"
                | "spawn_failed"
                | "wait_failed"
                | "output_read_failed"
                | "status_unavailable"
                | "patch_context_mismatch"
                | "invalid_patch"
                | "invalid_patch_size"
                | "empty_patch"
                | "invalid_patch_stat"
                | "duplicate_patch_path"
                | "rename_not_supported"
                | "invalid_precondition_hashes"
                | "precondition_path_not_in_patch"
                | "invalid_precondition_hash"
                | "patch_precondition_failed"
        )
    })
}

fn structured_read_permission_denial_is_terminal(normalized_skill: &str, err: &str) -> bool {
    let Some(structured) = crate::skills::parse_structured_skill_error(err) else {
        return false;
    };
    if structured.error_kind != "permission_denied" {
        return false;
    }
    let effective_skill = if structured.skill.trim().is_empty() {
        normalized_skill
    } else {
        structured.skill.as_str()
    };
    matches!(
        effective_skill.to_ascii_lowercase().as_str(),
        "fs_basic" | "system_basic" | "read_file" | "list_dir"
    )
}

fn run_cmd_error_is_observable(normalized_skill: &str, err: &str) -> bool {
    if crate::skills::is_observable_run_cmd_error(normalized_skill, err) {
        return true;
    }
    if !normalized_skill.eq_ignore_ascii_case("run_cmd") {
        return false;
    }
    let err = err.to_ascii_lowercase();
    err.contains("command failed")
        || err.contains("exit code")
        || err.contains("command not found")
        || err.contains("timed out")
        || err.contains("timeout")
}

pub(crate) fn classify_skill_failure_recovery(
    state: &AppState,
    actions: &[AgentAction],
    current_idx: usize,
    max_actions_per_turn: usize,
    normalized_skill: &str,
    call_args: Option<&Value>,
    err: &str,
) -> Option<&'static str> {
    if structured_read_permission_denial_is_terminal(normalized_skill, err) {
        return Some("recoverable_failure_finalize");
    }
    if crate::skills::is_crypto_account_access_error(normalized_skill, err) {
        return Some("recoverable_failure_finalize");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && run_cmd_is_literal_user_command(call_args)
        && !run_cmd_literal_failure_is_repairable(call_args)
        && remaining_actions_are_discussion_only(actions, current_idx, max_actions_per_turn)
    {
        return Some("recoverable_failure_finalize");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && has_remaining_action_after(actions, current_idx, max_actions_per_turn)
        && !remaining_actions_are_discussion_only(actions, current_idx, max_actions_per_turn)
        && !run_cmd_should_continue_after_split_failure(call_args)
    {
        if run_cmd_is_literal_user_command(call_args) {
            return Some("recoverable_failure_finalize");
        }
        return Some("recoverable_failure_continue_round");
    }
    if crate::skills::is_recoverable_skill_error(normalized_skill, err) {
        if has_remaining_action_after(actions, current_idx, max_actions_per_turn)
            && !remaining_actions_are_discussion_only(actions, current_idx, max_actions_per_turn)
        {
            return Some("recoverable_failure_continue_in_round");
        }
        if crate::skills::is_missing_target_skill_error(normalized_skill, err) {
            return Some("recoverable_failure_continue_round");
        }
        if remaining_actions_after_plan_capacity_are_discussion_only(
            actions,
            current_idx,
            max_actions_per_turn,
        ) {
            return Some("recoverable_failure_finalize");
        }
        if remaining_actions_are_discussion_only(actions, current_idx, max_actions_per_turn) {
            return Some("recoverable_failure_continue_round");
        }
        return Some("recoverable_failure_continue_round");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_should_continue_after_split_failure(call_args)
        && has_remaining_action_after(actions, current_idx, max_actions_per_turn)
    {
        return Some("recoverable_failure_continue_in_round");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && !has_remaining_action_after(actions, current_idx, max_actions_per_turn)
    {
        if current_idx > 0 && !has_remaining_action_after_full(actions, current_idx) {
            return Some("recoverable_failure_finalize");
        }
        if remaining_actions_after_plan_capacity_are_discussion_only(
            actions,
            current_idx,
            max_actions_per_turn,
        ) {
            return Some("recoverable_failure_finalize");
        }
        if run_cmd_is_literal_user_command(call_args)
            && run_cmd_literal_failure_is_repairable(call_args)
        {
            return Some("recoverable_failure_continue_round");
        }
        if !run_cmd_is_literal_user_command(call_args)
            && !run_cmd_should_continue_after_split_failure(call_args)
        {
            return Some("recoverable_failure_continue_round");
        }
        return Some("recoverable_failure_finalize");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && !run_cmd_is_literal_user_command(call_args)
        && !run_cmd_should_continue_after_split_failure(call_args)
        && remaining_actions_are_discussion_only(actions, current_idx, max_actions_per_turn)
    {
        return Some("recoverable_failure_continue_round");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && run_cmd_is_literal_user_command(call_args)
        && run_cmd_literal_failure_is_repairable(call_args)
        && remaining_actions_are_discussion_only(actions, current_idx, max_actions_per_turn)
    {
        return Some("recoverable_failure_continue_round");
    }
    if planner_can_repair_structured_skill_error(err) {
        if has_remaining_action_after(actions, current_idx, max_actions_per_turn)
            && !remaining_actions_are_discussion_only(actions, current_idx, max_actions_per_turn)
        {
            return Some("recoverable_failure_continue_in_round");
        }
        return Some("recoverable_failure_continue_round");
    }
    if state.skill_is_retryable(normalized_skill)
        && !state.skill_invocation_requires_confirmation_policy(normalized_skill, call_args)
    {
        if has_remaining_action_after(actions, current_idx, max_actions_per_turn) {
            return Some("recoverable_failure_continue_in_round");
        }
        if remaining_actions_are_discussion_only(actions, current_idx, max_actions_per_turn) {
            return Some("recoverable_failure_finalize");
        }
        return Some("recoverable_failure_continue_round");
    }
    if has_remaining_action_after(actions, current_idx, max_actions_per_turn)
        && call_args
            .map(|args| is_read_only_skill_invocation(state, normalized_skill, args))
            .unwrap_or(false)
    {
        return Some("recoverable_failure_continue_in_round");
    }
    if remaining_actions_are_discussion_only(actions, current_idx, max_actions_per_turn) {
        return Some("recoverable_failure_continue_in_round");
    }
    if remaining_actions_after_plan_capacity_are_discussion_only(
        actions,
        current_idx,
        max_actions_per_turn,
    ) {
        return Some("recoverable_failure_finalize");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && current_idx > 0
        && !has_remaining_action_after_full(actions, current_idx)
    {
        return Some("recoverable_failure_finalize");
    }
    None
}

fn is_read_only_skill_invocation(state: &AppState, normalized_skill: &str, args: &Value) -> bool {
    if state.skill_is_read_only(normalized_skill) {
        return true;
    }
    match normalized_skill {
        "read_file" | "list_dir" | "fs_search" | "system_basic" | "log_analyze" | "doc_parse"
        | "git_basic" | "http_basic" | "stock" | "weather" | "web_search_extract"
        | "health_check" | "task_control" => true,
        "db_basic" => args
            .get("action")
            .and_then(|v| v.as_str())
            .map(|a| {
                a.eq_ignore_ascii_case("sqlite_query")
                    || a.eq_ignore_ascii_case("schema_version")
                    || a.eq_ignore_ascii_case("sqlite_schema_version")
            })
            .unwrap_or(true),
        _ => false,
    }
}

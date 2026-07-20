use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, info};

use super::{
    append_delivery_message, append_progress_hint, build_safe_skill_args_summary,
    encode_progress_i18n, execute_prepared_skill_action, normalize_skill_arg_aliases,
    register_step_output, resolve_arg_string, resolve_arg_value,
    rewrite_args_with_auto_locator_path, rewrite_run_cmd_with_written_aliases,
    rewrite_tool_path_with_written_aliases, ActionLoopDecision, AgentLoopGuardPolicy,
    AgentRunContext, AppState, ClaimedTask, LoopState, RespondActionOutcome, SkillActionOutcome,
    WriteFileEffectivePath, PROGRESS_ARGS_SUMMARY_MAX_LEN,
};
use crate::{AgentAction, OutputResponseShape};

#[path = "dispatch_local_code_projection_gate.rs"]
mod dispatch_local_code_projection_gate;
#[path = "dispatch_synthesis.rs"]
mod dispatch_synthesis;
#[path = "dispatch_synthesis_bounded_read.rs"]
mod dispatch_synthesis_bounded_read;
#[path = "dispatch_support/execution_status.rs"]
mod execution_status;
#[path = "dispatch_support/failure_recovery.rs"]
mod failure_recovery;
#[path = "dispatch_support/respond_template_guard.rs"]
mod respond_template_guard;
#[path = "dispatch_support/skill_call_args.rs"]
mod skill_call_args;
#[path = "dispatch_support/status_answer.rs"]
mod status_answer;

use dispatch_local_code_projection_gate::{
    local_code_strict_json_projection_should_defer_observed_synthesis as gate_defer_observed_synthesis,
    local_code_strict_json_projection_should_defer_until_validation as gate_defer_until_validation,
    LOCAL_CODE_PROJECTION_PENDING_READBACK, LOCAL_CODE_PROJECTION_PENDING_VALIDATION,
};
use dispatch_synthesis::{
    local_code_task_strict_json_projection, requested_local_code_json_fields,
    route_resolved_intent, step_has_observable_synthesis_fact,
    strict_json_projection_answer_satisfies_request, synthesize_answer_allows_direct_fallback,
    synthesize_direct_fallback_would_passthrough_multiline_read_range,
    synthesize_direct_observed_fallback_answer,
    synthesize_evidence_policy_direct_observed_fallback_answer, synthesize_failure_observed_facts,
    synthesize_failure_should_replan, synthesize_route_allows_direct_fallback,
    synthesize_route_prefers_model_language_failure_answer,
};
use dispatch_synthesis_bounded_read::synthesize_bounded_read_range_direct_answer;
pub(super) use execution_status::deterministic_observed_execution_status_answer;
#[cfg(test)]
pub(super) use failure_recovery::active_recipe_terminal_discussion_should_replan;
#[cfg(not(test))]
use failure_recovery::active_recipe_terminal_discussion_should_replan;
pub(super) use failure_recovery::classify_skill_failure_recovery;
use failure_recovery::has_remaining_action_after;
use failure_recovery::record_active_recipe_terminal_discussion_replan;
use skill_call_args::{
    apply_recipe_run_cmd_overrides, read_file_requested_path,
    record_latest_run_cmd_command_output_vars, strip_internal_execution_args,
    strip_unsupported_planner_metadata_args, write_file_effective_path,
};
pub(super) use skill_call_args::{
    record_successful_run_cmd_command_output_var, successful_run_cmd_command_for_step,
};

pub(super) fn local_code_strict_json_projection_should_defer_observed_synthesis(
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    gate_defer_observed_synthesis(user_text, loop_state, agent_run_context)
}

pub(super) fn local_code_strict_json_projection_should_defer_until_validation(
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    gate_defer_until_validation(user_text, loop_state, agent_run_context)
}

pub(super) fn local_code_strict_json_projection_from_machine_evidence(
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let answer = local_code_task_strict_json_projection(user_text, loop_state, agent_run_context)?;
    strict_json_projection_answer_satisfies_request(
        user_text,
        &answer,
        loop_state,
        agent_run_context,
    )
    .then_some(answer)
}

pub(super) fn local_code_strict_json_answer_satisfies_request(
    user_text: &str,
    answer: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    strict_json_projection_answer_satisfies_request(
        user_text,
        answer,
        loop_state,
        agent_run_context,
    )
}

pub(super) fn apply_skill_action_outcome(
    loop_state: &mut LoopState,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    outcome: SkillActionOutcome,
) -> ActionLoopDecision {
    *ended_with_user_visible_output |= outcome.ended_with_user_visible_output;
    *executed_actions += 1;
    loop_state.total_steps_executed += 1;
    if outcome.continue_in_round {
        return ActionLoopDecision::ContinueRound;
    }
    if let Some(reason) = outcome.stop_signal {
        return ActionLoopDecision::StopRound(reason);
    }
    ActionLoopDecision::NextAction
}

pub(super) fn apply_respond_action_outcome(
    loop_state: &mut LoopState,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    outcome: RespondActionOutcome,
) -> ActionLoopDecision {
    *ended_with_user_visible_output |= outcome.ended_with_user_visible_output;
    *executed_actions += 1;
    loop_state.total_steps_executed += 1;
    if outcome.should_stop {
        return ActionLoopDecision::StopRound(outcome.stop_signal.unwrap_or_default());
    }
    ActionLoopDecision::NextAction
}

fn unresolved_capability_error(state: &AppState, capability: &str, args: &Value) -> String {
    let (_resolved, record) =
        crate::capability_resolver::resolve_capability_action_with_record_for_state(
            state,
            capability,
            args.clone(),
        );
    json!({
        "error_kind": record.reason_code,
        "reason_code": record.reason_code,
        "message_key": record.reason_code,
        "owner_layer": record.owner_layer,
        "outcome": record.outcome,
        "source": record.source,
        "capability_ref": record.capability_ref,
        "resolved_ref": record.resolved_ref,
        "planner_kind": record.planner_kind,
    })
    .to_string()
}

fn rewrite_response_with_written_aliases(text: &str, loop_state: &LoopState) -> String {
    let mut out = text.to_string();
    for (alias, effective) in &loop_state.written_file_aliases {
        let file_alias = format!("FILE:{alias}");
        let file_effective = format!("FILE:{effective}");
        let image_alias = format!("IMAGE_FILE:{alias}");
        let image_effective = format!("IMAGE_FILE:{effective}");
        out = out.replace(&file_alias, &file_effective);
        out = out.replace(&image_alias, &image_effective);
        let trimmed = out.trim();
        if trimmed == alias {
            return effective.clone();
        }
        if trimmed == format!("`{alias}`") {
            return effective.clone();
        }
    }
    out
}

fn synthesize_failure_default_text(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
) -> String {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let default_payload =
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty.machine_default_payload();
    crate::i18n_t_for_language_hint_with_default_vars(
        state,
        &language_hint,
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty.i18n_key(),
        &default_payload,
        &[],
    )
}

async fn synthesize_failure_user_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    refs_summary: &str,
) -> String {
    let default_text = synthesize_failure_default_text(state, task, user_text);
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let has_observed_result = loop_state
        .executed_step_results
        .iter()
        .any(step_has_observable_synthesis_fact);
    let mut policy_boundary = vec![
        "task_success_claim_allowed=false".to_string(),
        "expose_internal_details=false".to_string(),
        "response_scope=observed_synthesis_failure_only".to_string(),
        "missing_result_invention_allowed=false".to_string(),
    ];
    if has_observed_result {
        policy_boundary.push("observed_execution_results_available=true".to_string());
        policy_boundary.push("raw_results_or_retry_synthesis_available=true".to_string());
    } else {
        policy_boundary.push("usable_execution_result_available=false".to_string());
    }
    let contract = crate::fallback::UserResponseContract::tool_failure(
        if has_observed_result {
            "synthesize_answer_no_publishable_answer"
        } else {
            "synthesize_answer_no_evidence"
        },
        user_text,
        &route_resolved_intent(agent_run_context),
        synthesize_failure_observed_facts(loop_state, refs_summary),
        policy_boundary,
        if has_observed_result {
            "brief_failure_with_next_step"
        } else {
            "brief_failure"
        },
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
        &default_text,
    )
    .await
}

#[cfg(test)]
#[path = "dispatch_support_tests.rs"]
mod tests;

fn route_requires_file_token_delivery(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .map(|route| {
            route.delivery_required
                || matches!(route.response_shape, OutputResponseShape::FileToken)
        })
        .unwrap_or(false)
}

fn file_token_payload_contains_runtime_artifact(payload: &str) -> bool {
    let payload = payload.trim();
    if payload.is_empty() {
        return true;
    }
    if Path::new(payload).is_file() {
        return false;
    }
    payload.contains("{{")
        || payload.contains("}}")
        || payload.contains('\n')
        || payload.starts_with('{')
        || payload.starts_with('[')
        || payload.contains("\"action\"")
        || payload.contains("\"counts\"")
        || payload.contains("\"names\"")
        || payload.contains("\"results\"")
}

fn unresolved_file_token_delivery_artifact(text: &str) -> bool {
    crate::extract_delivery_file_tokens(text)
        .iter()
        .filter_map(|token| crate::finalize::parse_delivery_file_token(token.trim()))
        .any(|(_kind, payload)| file_token_payload_contains_runtime_artifact(payload))
}

pub(super) fn handle_respond_action(
    state: &AppState,
    task: &ClaimedTask,
    actions: &[AgentAction],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    global_step: usize,
    step_in_round: usize,
    fingerprint: &str,
    content: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> RespondActionOutcome {
    let resolved_text = rewrite_response_with_written_aliases(
        &resolve_arg_string(content, loop_state).trim().to_string(),
        loop_state,
    )
    .trim()
    .to_string();
    let has_remaining_actions = has_remaining_action_after(actions, idx, policy.max_steps);
    let terminal_last_output_passthrough = !has_remaining_actions
        && !resolved_text.is_empty()
        && respond_template_guard::bare_last_output_placeholder(content);
    let text = terminal_last_output_passthrough
        .then(|| {
            loop_state
                .last_publishable_synthesis_output
                .as_deref()
                .map(str::trim)
                .filter(|answer| !answer.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    respond_template_guard::terminal_last_output_machine_projection(loop_state)
                })
        })
        .flatten()
        .unwrap_or(resolved_text);

    if let Some(outcome) = respond_template_guard::unresolved_runtime_template_respond_outcome(
        state,
        task,
        loop_state,
        global_step,
        step_in_round,
        content,
        &text,
    ) {
        return outcome;
    }

    if active_recipe_terminal_discussion_should_replan(actions, loop_state, policy, idx) {
        record_active_recipe_terminal_discussion_replan(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            "respond",
        );
        return RespondActionOutcome {
            ended_with_user_visible_output: false,
            stop_signal: Some("recoverable_failure_continue_round".to_string()),
            should_stop: true,
        };
    }

    if route_requires_file_token_delivery(agent_run_context)
        && unresolved_file_token_delivery_artifact(&text)
    {
        let error = "invalid file delivery token: runtime observation was embedded into FILE path";
        loop_state.has_recoverable_failure_context = true;
        super::attempt_ledger::record_attempt_with_retry_instruction(
            loop_state,
            "respond",
            &format!("content={}", crate::truncate_for_agent_trace(&text)),
            crate::executor::StepExecutionStatus::Error,
            &text,
            Some("invalid_delivery_token"),
            error,
            Some(
                "Use the already observed structured output to select a concrete existing file path, or run one bounded command/tool that directly returns that selected path. Then respond with exactly FILE:<path>; do not put {{last_output}} or a structured object inside FILE:.",
            ),
        );
        crate::append_subtask_result(
            &mut loop_state.subtask_results,
            global_step,
            "respond",
            false,
            error,
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
                step_id: format!("step_{}", global_step),
                skill: "respond".to_string(),
                status: crate::executor::StepExecutionStatus::Error,
                output: None,
                error: Some(error.to_string()),
                started_at: 0,
                finished_at: 0,
            });
        loop_state.history_compact.push(format!(
            "round={} step={} respond invalid_delivery_token",
            loop_state.round_no, step_in_round
        ));
        info!(
            "respond_invalid_delivery_token_replan task_id={} round={} step={} text={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            crate::truncate_for_log(&text)
        );
        return RespondActionOutcome {
            ended_with_user_visible_output: false,
            stop_signal: Some("recoverable_failure_continue_round".to_string()),
            should_stop: true,
        };
    }

    let terminal_direct_answer =
        !has_remaining_actions && !text.is_empty() && !loop_state.has_tool_or_skill_output;
    let duplicate_delivery = loop_state
        .delivery_messages
        .last()
        .is_some_and(|last| last.trim() == text.trim());
    let publish_respond = respond_template_guard::should_publish_respond_message(loop_state, &text)
        || ((terminal_direct_answer || terminal_last_output_passthrough) && !duplicate_delivery);
    if !text.is_empty() && (publish_respond || !has_remaining_actions) {
        loop_state.last_user_visible_respond = Some(text.clone());
    }
    if publish_respond {
        crate::append_subtask_result(
            &mut loop_state.subtask_results,
            global_step,
            "respond",
            true,
            &text,
        );
        append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            text.clone(),
        );
        info!(
            "delivery appended from respond task_id={} len={} has_remaining={}",
            task.task_id,
            loop_state.delivery_messages.len(),
            has_remaining_actions
        );
        let hint = encode_progress_i18n("telegram.progress.reply_generated", &[]);
        append_progress_hint(state, task, &mut loop_state.progress_messages, hint);
    }
    if !publish_respond && !text.is_empty() {
        debug!(
            "executor_step_skip task_id={} round={} step={} type=respond reason=respond_not_publishable trace_only",
            task.task_id, loop_state.round_no, step_in_round
        );
    }
    register_step_output(loop_state, global_step, step_in_round, "respond", &text);
    if !text.is_empty() {
        loop_state
            .executed_step_results
            .push(crate::executor::StepExecutionResult {
                step_id: format!("step_{global_step}"),
                skill: "respond".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output: Some(text.clone()),
                error: None,
                started_at: 0,
                finished_at: 0,
            });
    }
    *loop_state
        .successful_action_fingerprints
        .entry(fingerprint.to_string())
        .or_insert(0) += 1;
    info!(
        "executor_result_ok task_id={} round={} step={} type=respond output={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        crate::truncate_for_log(&text)
    );
    loop_state.history_compact.push(format!(
        "round={} step={} respond{}",
        loop_state.round_no,
        step_in_round,
        if has_remaining_actions {
            "_intermediate"
        } else {
            ""
        }
    ));
    RespondActionOutcome {
        ended_with_user_visible_output: publish_respond
            && !has_remaining_actions
            && !text.is_empty(),
        stop_signal: if has_remaining_actions {
            None
        } else {
            Some("respond".to_string())
        },
        should_stop: !has_remaining_actions,
    }
}

pub(super) async fn handle_call_tool_action(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    action: &AgentAction,
    fingerprint: &str,
    global_step: usize,
    step_in_round: usize,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    tool: &str,
    args: &Value,
    action_trace_kind: &'static str,
) -> Result<ActionLoopDecision, String> {
    let mut resolved_args = resolve_arg_value(args, loop_state);
    let mut normalized_skill = state.resolve_canonical_skill_name(tool);
    if normalize_skill_arg_aliases(&normalized_skill, &mut resolved_args) {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=arg_alias skill={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    let requested_virtual_skill = normalized_skill.clone();
    let requested_virtual_args = resolved_args.clone();
    if let Some(rewrite) =
        crate::virtual_tools::rewrite_virtual_tool_call(&normalized_skill, resolved_args.clone())?
    {
        info!(
            "executor_virtual_tool_rewrite task_id={} round={} step={} requested_tool={} runtime_tool={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            rewrite.runtime_tool,
            crate::truncate_for_log(&rewrite.runtime_args.to_string())
        );
        normalized_skill = state.resolve_canonical_skill_name(&rewrite.runtime_tool);
        resolved_args = rewrite.runtime_args;
        if crate::agent_engine::enrich_scratch_filesystem_cleanup_runtime_args(
            state,
            loop_state,
            &requested_virtual_skill,
            &requested_virtual_args,
            &normalized_skill,
            &mut resolved_args,
        ) {
            info!(
                "executor_args_rewrite task_id={} round={} step={} type=scratch_cleanup_directory skill={} args={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_log(&resolved_args.to_string())
            );
        }
        if normalize_skill_arg_aliases(&normalized_skill, &mut resolved_args) {
            info!(
                "executor_args_rewrite task_id={} round={} step={} type=runtime_arg_alias skill={} args={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_log(&resolved_args.to_string())
            );
        }
    }
    if rewrite_args_with_auto_locator_path(&normalized_skill, &mut resolved_args, loop_state) {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=auto_locator skill={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    let read_file_requested_path = read_file_requested_path(&normalized_skill, &resolved_args);
    let write_file_effective_path =
        write_file_effective_path(state, &normalized_skill, &resolved_args);
    if normalized_skill == "run_cmd" {
        if let Some(obj) = resolved_args.as_object_mut() {
            if let Some(command) = obj.get("command").and_then(|v| v.as_str()) {
                let rewritten = rewrite_run_cmd_with_written_aliases(command, loop_state);
                if rewritten != command {
                    obj.insert("command".to_string(), Value::String(rewritten));
                }
            }
        }
    }
    rewrite_tool_path_with_written_aliases(&normalized_skill, &mut resolved_args, loop_state);
    apply_recipe_run_cmd_overrides(
        state,
        loop_state,
        policy,
        &normalized_skill,
        &mut resolved_args,
    );
    let recovery_args = resolved_args.clone();
    strip_internal_execution_args(&mut resolved_args);
    let removed_metadata =
        strip_unsupported_planner_metadata_args(state, &normalized_skill, &mut resolved_args);
    if !removed_metadata.is_empty() {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=planner_metadata_strip skill={} removed={:?} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            removed_metadata,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    record_latest_run_cmd_command_output_vars(loop_state, &normalized_skill, &resolved_args);
    loop_state.tool_calls_total += 1;
    let args_summary = build_safe_skill_args_summary(&resolved_args, PROGRESS_ARGS_SUMMARY_MAX_LEN);
    let skill_outcome = execute_prepared_skill_action(
        state,
        task,
        goal,
        user_text,
        actions,
        round_steps,
        loop_state,
        policy,
        idx,
        action,
        fingerprint,
        global_step,
        step_in_round,
        &normalized_skill,
        &requested_virtual_skill,
        &requested_virtual_args,
        resolved_args,
        Some(recovery_args),
        write_file_effective_path,
        read_file_requested_path,
        args_summary,
        action_trace_kind,
    )
    .await?;
    Ok(apply_skill_action_outcome(
        loop_state,
        executed_actions,
        ended_with_user_visible_output,
        skill_outcome,
    ))
}

pub(super) async fn handle_call_skill_action(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    action: &AgentAction,
    fingerprint: &str,
    global_step: usize,
    step_in_round: usize,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    skill: &str,
    args: &Value,
    action_trace_kind: &'static str,
) -> Result<ActionLoopDecision, String> {
    let mut resolved_args = resolve_arg_value(args, loop_state);
    loop_state.tool_calls_total += 1;
    let mut normalized_skill = state.resolve_canonical_skill_name(skill);
    if normalize_skill_arg_aliases(&normalized_skill, &mut resolved_args) {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=arg_alias skill={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    let requested_virtual_skill = normalized_skill.clone();
    let requested_virtual_args = resolved_args.clone();
    if let Some(rewrite) =
        crate::virtual_tools::rewrite_virtual_tool_call(&normalized_skill, resolved_args.clone())?
    {
        info!(
            "executor_virtual_tool_rewrite task_id={} round={} step={} requested_tool={} runtime_tool={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            rewrite.runtime_tool,
            crate::truncate_for_log(&rewrite.runtime_args.to_string())
        );
        normalized_skill = state.resolve_canonical_skill_name(&rewrite.runtime_tool);
        resolved_args = rewrite.runtime_args;
        if crate::agent_engine::enrich_scratch_filesystem_cleanup_runtime_args(
            state,
            loop_state,
            &requested_virtual_skill,
            &requested_virtual_args,
            &normalized_skill,
            &mut resolved_args,
        ) {
            info!(
                "executor_args_rewrite task_id={} round={} step={} type=scratch_cleanup_directory skill={} args={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_log(&resolved_args.to_string())
            );
        }
        if normalize_skill_arg_aliases(&normalized_skill, &mut resolved_args) {
            info!(
                "executor_args_rewrite task_id={} round={} step={} type=runtime_arg_alias skill={} args={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_log(&resolved_args.to_string())
            );
        }
    }
    if rewrite_args_with_auto_locator_path(&normalized_skill, &mut resolved_args, loop_state) {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=auto_locator skill={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    apply_recipe_run_cmd_overrides(
        state,
        loop_state,
        policy,
        &normalized_skill,
        &mut resolved_args,
    );
    let recovery_args = resolved_args.clone();
    strip_internal_execution_args(&mut resolved_args);
    let removed_metadata =
        strip_unsupported_planner_metadata_args(state, &normalized_skill, &mut resolved_args);
    if !removed_metadata.is_empty() {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=planner_metadata_strip skill={} removed={:?} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            removed_metadata,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    record_latest_run_cmd_command_output_vars(loop_state, &normalized_skill, &resolved_args);
    let read_file_requested_path = read_file_requested_path(&normalized_skill, &resolved_args);
    let write_file_effective_path =
        write_file_effective_path(state, &normalized_skill, &resolved_args);
    let args_summary = build_safe_skill_args_summary(&resolved_args, PROGRESS_ARGS_SUMMARY_MAX_LEN);
    let skill_outcome = execute_prepared_skill_action(
        state,
        task,
        goal,
        user_text,
        actions,
        round_steps,
        loop_state,
        policy,
        idx,
        action,
        fingerprint,
        global_step,
        step_in_round,
        &normalized_skill,
        &requested_virtual_skill,
        &requested_virtual_args,
        resolved_args,
        Some(recovery_args),
        write_file_effective_path,
        read_file_requested_path,
        args_summary,
        action_trace_kind,
    )
    .await?;
    Ok(apply_skill_action_outcome(
        loop_state,
        executed_actions,
        ended_with_user_visible_output,
        skill_outcome,
    ))
}

pub(super) async fn handle_synthesize_answer_action(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    action: &AgentAction,
    global_step: usize,
    step_in_round: usize,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    agent_run_context: Option<&AgentRunContext>,
    evidence_refs: &[String],
) -> Result<ActionLoopDecision, String> {
    loop_state.tool_calls_total += 1;
    let refs_summary = if evidence_refs.is_empty() {
        "last_output".to_string()
    } else {
        evidence_refs.join(",")
    };
    info!(
        "{} executor_step_execute task_id={} round={} step={} type=synthesize_answer refs={}",
        crate::highlight_tag("llm"),
        task.task_id,
        loop_state.round_no,
        step_in_round,
        crate::truncate_for_log(&refs_summary)
    );
    let capability_synthesis =
        match super::capability_result_synthesis::synthesize_from_capability_results(
            state,
            task,
            user_text,
            loop_state,
            agent_run_context,
        )
        .await
        {
            Ok(synthesis) => synthesis,
            Err(error_code) => {
                tracing::warn!(
                    "capability_result_synthesis_unavailable task_id={} error_code={}",
                    task.task_id,
                    error_code
                );
                None
            }
        };
    let capability_synthesis_answer = capability_synthesis
        .as_ref()
        .map(|synthesis| synthesis.answer.clone());
    let step_execution =
        crate::executor::execute_step(&format!("step_{global_step}"), action, || async {
            if let Some(answer) = capability_synthesis_answer {
                return Ok(answer);
            }
            if let Some(answer) =
                synthesize_bounded_read_range_direct_answer(loop_state, agent_run_context)
            {
                return Ok(answer);
            }
            if let Some(answer) = synthesize_evidence_policy_direct_observed_fallback_answer(
                state,
                loop_state,
                agent_run_context,
            ) {
                return Ok(answer);
            }
            if let Some(answer) =
                crate::agent_engine::observed_output::direct_answer_from_referenced_observation_i18n(
                    loop_state,
                    state,
                    agent_run_context,
                    evidence_refs,
                )
            {
                return Ok(answer);
            }
            if !synthesize_route_prefers_model_language_failure_answer(agent_run_context) {
                if let Some(answer) = deterministic_observed_execution_status_answer(
                    state,
                    task,
                    user_text,
                    loop_state,
                    agent_run_context,
                ) {
                    return Ok(answer);
                }
            }
            if let Some((answer, _summary)) =
                crate::finalize::deterministic_matrix_observed_shape_answer(
                    state,
                    task,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
            {
                return Ok(answer);
            }
            let requires_synthesized_delivery = agent_run_context
                .and_then(|context| context.output_contract())
                .is_some_and(
                    crate::agent_engine::observed_output::route_requires_synthesized_delivery,
                );
            let direct_fallback_blocked =
                synthesize_direct_fallback_would_passthrough_multiline_read_range(
                    loop_state,
                    agent_run_context,
                );
            let allow_direct_fallback = synthesize_answer_allows_direct_fallback(evidence_refs)
                && synthesize_route_allows_direct_fallback(agent_run_context)
                && !direct_fallback_blocked;
            if let Some(answer) =
                local_code_strict_json_projection_from_machine_evidence(
                    user_text,
                    loop_state,
                    agent_run_context,
                )
            {
                return Ok(answer);
            }
            if local_code_strict_json_projection_should_defer_observed_synthesis(
                user_text,
                loop_state,
                agent_run_context,
            ) {
                return Err(LOCAL_CODE_PROJECTION_PENDING_READBACK.to_string());
            }
            if local_code_strict_json_projection_should_defer_until_validation(
                user_text,
                loop_state,
                agent_run_context,
            ) {
                return Err(LOCAL_CODE_PROJECTION_PENDING_VALIDATION.to_string());
            }
            if allow_direct_fallback {
                if let Some(answer) =
                    synthesize_direct_observed_fallback_answer(state, loop_state, agent_run_context)
                {
                    return Ok(answer);
                }
            }
            let synthesized =
                crate::agent_engine::observed_output::synthesize_answer_from_observed_output(
                    state,
                    task,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
                .await
                .and_then(|(answer, summary)| {
                    (summary.disposition
                        == Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
                        && summary.completion_ok == Some(true))
                    .then_some(answer)
                })
                .filter(|answer| !answer.trim().is_empty());
            if let Some(answer) = synthesized {
                return Ok(answer);
            }
            if !allow_direct_fallback && !requires_synthesized_delivery && !direct_fallback_blocked
            {
                if let Some(answer) =
                    synthesize_direct_observed_fallback_answer(state, loop_state, agent_run_context)
                {
                    return Ok(answer);
                }
            }
            if synthesize_failure_should_replan(loop_state) {
                return Err("synthesize_answer_no_publishable_answer".to_string());
            }
            Err(synthesize_failure_user_message(
                state,
                task,
                user_text,
                loop_state,
                agent_run_context,
                &refs_summary,
            )
            .await)
        })
        .await;
    match step_execution
        .output
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(answer) => {
            let answer = answer.to_string();
            if let Some(synthesis) = capability_synthesis.as_ref() {
                loop_state.last_capability_synthesis_output = Some(answer.clone());
                loop_state.output_vars.insert(
                    "agent_loop.capability_synthesis_confidence".to_string(),
                    synthesis.confidence.to_string(),
                );
                loop_state.output_vars.insert(
                    "agent_loop.capability_synthesis_evidence_count".to_string(),
                    synthesis.evidence_count.to_string(),
                );
            }
            if strict_json_projection_answer_satisfies_request(
                user_text,
                &answer,
                loop_state,
                agent_run_context,
            ) {
                loop_state.output_vars.insert(
                    "agent_loop.strict_json_projection_publishable".to_string(),
                    "true".to_string(),
                );
                loop_state.output_vars.insert(
                    "agent_loop.strict_json_projection_output".to_string(),
                    answer.clone(),
                );
            } else {
                loop_state
                    .output_vars
                    .remove("agent_loop.strict_json_projection_publishable");
                loop_state
                    .output_vars
                    .remove("agent_loop.strict_json_projection_output");
            }
            crate::append_subtask_result(
                &mut loop_state.subtask_results,
                global_step,
                "synthesize_answer",
                true,
                &answer,
            );
            register_step_output(
                loop_state,
                global_step,
                step_in_round,
                "synthesize_answer",
                &answer,
            );
            loop_state.last_publishable_synthesis_output = Some(answer.clone());
            loop_state.history_compact.push(format!(
                "round={} step={} synthesize_answer ok refs={}",
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_agent_trace(&refs_summary)
            ));
            info!(
                "executor_result_ok task_id={} round={} step={} type=synthesize_answer output={} trace_only=raw_not_delivery",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_log(&answer)
            );
            loop_state.executed_step_results.push(step_execution);
            let outcome = SkillActionOutcome {
                ended_with_user_visible_output: true,
                stop_signal: None,
                continue_in_round: false,
            };
            Ok(apply_skill_action_outcome(
                loop_state,
                executed_actions,
                ended_with_user_visible_output,
                outcome,
            ))
        }
        None => {
            let err = step_execution
                .error
                .clone()
                .unwrap_or_else(|| "synthesize_answer failed".to_string());
            let local_code_projection_pending_readback =
                err == LOCAL_CODE_PROJECTION_PENDING_READBACK;
            let local_code_projection_pending_validation =
                err == LOCAL_CODE_PROJECTION_PENDING_VALIDATION;
            let should_replan = !local_code_projection_pending_readback
                && !local_code_projection_pending_validation
                && synthesize_failure_should_replan(loop_state);
            if should_replan {
                let compact_err = err
                    .replace('\n', " ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let compact_err = crate::truncate_for_agent_trace(&compact_err);
                append_progress_hint(
                    state,
                    task,
                    &mut loop_state.progress_messages,
                    encode_progress_i18n(
                        "telegram.progress.step_failed",
                        &[
                            ("step", &step_in_round.to_string()),
                            ("skill", "synthesize_answer"),
                            ("error", &compact_err),
                        ],
                    ),
                );
                append_progress_hint(
                    state,
                    task,
                    &mut loop_state.progress_messages,
                    encode_progress_i18n("telegram.progress.retry_replan", &[]),
                );
            }
            crate::append_subtask_result(
                &mut loop_state.subtask_results,
                global_step,
                "synthesize_answer",
                false,
                &err,
            );
            loop_state.history_compact.push(format!(
                "round={} step={} synthesize_answer failed error={}",
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_agent_trace(&err)
            ));
            loop_state.executed_step_results.push(step_execution);
            *executed_actions += 1;
            loop_state.total_steps_executed += 1;
            info!(
                "synthesize_answer_failed_defer_to_finalize task_id={} round={} step={} error={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_log(&err)
            );
            Ok(ActionLoopDecision::StopRound(if should_replan {
                "recoverable_failure_continue_round".to_string()
            } else if local_code_projection_pending_validation {
                "recoverable_failure_continue_round".to_string()
            } else if local_code_projection_pending_readback {
                LOCAL_CODE_PROJECTION_PENDING_READBACK.to_string()
            } else {
                "synthesize_answer_failed".to_string()
            }))
        }
    }
}

pub(super) async fn dispatch_round_action(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    action: &AgentAction,
    fingerprint: &str,
    global_step: usize,
    step_in_round: usize,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<ActionLoopDecision, String> {
    let resolved_from_call_capability = matches!(action, AgentAction::CallCapability { .. });
    let resolved_capability_action =
        if let AgentAction::CallCapability { capability, args } = action {
            let (resolved, record) =
                crate::capability_resolver::resolve_capability_action_with_record_for_state(
                    state,
                    capability,
                    args.clone(),
                );
            loop_state
                .task_observations
                .push(record.dispatch_observation(loop_state.round_no, global_step, step_in_round));
            resolved
        } else {
            None
        };
    let action = resolved_capability_action.as_ref().unwrap_or(action);
    match action {
        AgentAction::CallTool { tool, args } => {
            handle_call_tool_action(
                state,
                task,
                goal,
                user_text,
                actions,
                round_steps,
                loop_state,
                policy,
                idx,
                action,
                fingerprint,
                global_step,
                step_in_round,
                executed_actions,
                ended_with_user_visible_output,
                tool,
                args,
                if resolved_from_call_capability {
                    "call_capability"
                } else {
                    "call_tool_legacy"
                },
            )
            .await
        }
        AgentAction::CallSkill { skill, args } => {
            handle_call_skill_action(
                state,
                task,
                goal,
                user_text,
                actions,
                round_steps,
                loop_state,
                policy,
                idx,
                action,
                fingerprint,
                global_step,
                step_in_round,
                executed_actions,
                ended_with_user_visible_output,
                skill,
                args,
                if resolved_from_call_capability {
                    "call_capability"
                } else {
                    "call_skill"
                },
            )
            .await
        }
        AgentAction::CallCapability { capability, args } => {
            Err(unresolved_capability_error(state, capability, args))
        }
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            if active_recipe_terminal_discussion_should_replan(actions, loop_state, policy, idx) {
                record_active_recipe_terminal_discussion_replan(
                    state,
                    task,
                    loop_state,
                    global_step,
                    step_in_round,
                    "synthesize_answer",
                );
                *executed_actions += 1;
                loop_state.total_steps_executed += 1;
                return Ok(ActionLoopDecision::StopRound(
                    "recoverable_failure_continue_round".to_string(),
                ));
            }
            handle_synthesize_answer_action(
                state,
                task,
                user_text,
                loop_state,
                action,
                global_step,
                step_in_round,
                executed_actions,
                ended_with_user_visible_output,
                agent_run_context,
                evidence_refs,
            )
            .await
        }
        AgentAction::Respond { content } => {
            let respond_outcome = handle_respond_action(
                state,
                task,
                actions,
                loop_state,
                policy,
                idx,
                global_step,
                step_in_round,
                fingerprint,
                content,
                agent_run_context,
            );
            Ok(apply_respond_action_outcome(
                loop_state,
                executed_actions,
                ended_with_user_visible_output,
                respond_outcome,
            ))
        }
        AgentAction::Think { .. } => {
            *executed_actions += 1;
            loop_state.total_steps_executed += 1;
            Ok(ActionLoopDecision::NextAction)
        }
    }
}

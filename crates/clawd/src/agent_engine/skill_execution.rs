use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, info, warn};

use super::{
    build_resume_context_error, classify_skill_failure_recovery, ensure_task_running,
    register_failed_step_output, register_failed_step_structured_error_fields,
    register_file_path_output, register_step_output, remember_written_file_alias,
    AgentLoopGuardPolicy, AppState, ClaimedTask, LoopState, SkillActionOutcome,
    WriteFileEffectivePath, TASK_CANCELED_ERR,
};
use crate::{repo, run_skill_with_runner_outcome};

#[path = "skill_execution_auto_sudo.rs"]
mod skill_execution_auto_sudo;
#[path = "skill_execution_evidence.rs"]
mod skill_execution_evidence;
#[path = "skill_execution_observations.rs"]
mod skill_execution_observations;
#[path = "skill_execution_preflight.rs"]
mod skill_execution_preflight;
#[path = "skill_execution_subagent.rs"]
mod skill_execution_subagent;

#[cfg(test)]
use skill_execution_auto_sudo::build_auto_sudo_retry_args;
use skill_execution_auto_sudo::{
    publish_failure_recovery_progress, skill_error_observation_or_raw, skill_error_progress_token,
    try_auto_sudo_retry_after_permission_denied,
};
#[cfg(test)]
use skill_execution_evidence::admitted_extra_field_exists;
use skill_execution_evidence::{
    matrix_admitted_external_evidence_output, merge_isolation_artifact_refs,
    register_structured_extra_file_path_outputs, skill_extra_requests_user_input,
    structured_extra_evidence_output,
};
#[cfg(test)]
use skill_execution_observations::record_post_tool_use_observation;
use skill_execution_observations::{
    log_step_journal_summary, record_hook_evaluation_observation,
    record_mcp_tool_execution_observation, record_permission_request_hook,
    record_post_tool_use_hook_observations,
};
use skill_execution_preflight::{
    capability_isolation_artifact_refs, capability_isolation_policy_error,
    evidence_policy_action_policy_error, handle_preflight_argument_failure,
    structured_observation_path_argument_error, unresolved_runtime_template_argument_error,
    validate_skill_output_contract,
};
use skill_execution_subagent::{record_subagent_hook_stage, record_subagent_step_execution};

async fn run_mcp_tool_observation(
    state: &AppState,
    task: &ClaimedTask,
    capability: &str,
    args: Value,
) -> Result<(String, Value), String> {
    let outcome = state
        .core
        .mcp_runtime
        .call(
            capability,
            args,
            state.worker.task_cancellation_token(&task.task_id),
        )
        .await
        .map_err(|error| {
            json!({
                "error_code": error.code(),
                "message_key": error.code(),
                "adapter_kind": "mcp_tool",
            })
            .to_string()
        })?;
    let outcome_json = serde_json::to_value(&outcome).map_err(|_| {
        json!({
            "error_code": "mcp_result_serialize_failed",
            "message_key": "mcp_result_serialize_failed",
            "adapter_kind": "mcp_tool",
        })
        .to_string()
    })?;
    let raw = serde_json::to_string(&outcome).map_err(|_| {
        json!({
            "error_code": "mcp_result_serialize_failed",
            "message_key": "mcp_result_serialize_failed",
            "adapter_kind": "mcp_tool",
        })
        .to_string()
    })?;
    Ok((
        raw,
        json!({
            "adapter_kind": "mcp_tool",
            "mcp_result": outcome_json,
        }),
    ))
}

#[cfg(test)]
use skill_execution_preflight::{
    contains_unresolved_runtime_template_arg, preflight_failure_metadata,
    preflight_permission_decision,
};

fn remember_skill_metadata(loop_state: &mut LoopState, normalized_skill: &str) {
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), normalized_skill.to_string());
}

async fn handle_skill_step_success(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    fingerprint: &str,
    step_execution: &crate::executor::StepExecutionResult,
    global_step: usize,
    step_in_round: usize,
    normalized_skill: &str,
    action_trace_kind: &str,
    args_summary: &str,
    action_args: &Value,
    out: &str,
    action_effect: crate::execution_recipe::ActionEffect,
    validation_observation: crate::execution_recipe::ValidationObservation,
    structured_extra: Option<&Value>,
    write_file_effective_path: Option<&WriteFileEffectivePath>,
    read_file_requested_path: Option<&str>,
) -> Result<SkillActionOutcome, String> {
    ensure_task_running(state, task)?;
    remember_skill_metadata(loop_state, normalized_skill);
    crate::execution_recipe::apply_target_scope_progress(
        &mut loop_state.execution_recipe,
        state,
        normalized_skill,
        action_args,
        true,
    );
    if let Err(contract_err) = validate_skill_output_contract(state, normalized_skill, out) {
        warn!(
            "skill_output_contract_mismatch task_id={} round={} step={} skill={} err={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&contract_err)
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} output_contract_mismatch={}",
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_agent_trace(&contract_err)
        ));
    }
    if let Some((original_path, _effective_path, user_visible_path)) = write_file_effective_path {
        remember_written_file_alias(loop_state, original_path, user_visible_path);
        register_file_path_output(
            loop_state,
            global_step,
            step_in_round,
            &format!("skill.{normalized_skill}"),
            user_visible_path,
        );
    } else if let Some(path) = read_file_requested_path {
        register_file_path_output(
            loop_state,
            global_step,
            step_in_round,
            &format!("skill.{normalized_skill}"),
            path,
        );
    }
    register_structured_extra_file_path_outputs(
        loop_state,
        global_step,
        step_in_round,
        normalized_skill,
        structured_extra,
    );
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        &format!("skill({normalized_skill})"),
        true,
        out,
    );
    let mut stop_signal = None;
    let mut mark_successful_fingerprint = true;
    match &validation_observation {
        crate::execution_recipe::ValidationObservation::Passed => {
            record_latest_validation_result(
                loop_state,
                normalized_skill,
                global_step,
                step_in_round,
                "passed",
                "validation_passed",
                action_effect,
            );
            crate::execution_recipe::apply_action_effect_success(
                &mut loop_state.execution_recipe,
                action_effect,
            );
            super::maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
        }
        crate::execution_recipe::ValidationObservation::Failed(detail) => {
            record_latest_validation_result(
                loop_state,
                normalized_skill,
                global_step,
                step_in_round,
                "failed",
                "validation_failed",
                action_effect,
            );
            crate::execution_recipe::apply_action_effect_failure(
                &mut loop_state.execution_recipe,
                action_effect,
            );
            register_failed_step_output(
                loop_state,
                global_step,
                step_in_round,
                &format!("skill.{normalized_skill}"),
                &format!("skill({normalized_skill})"),
                detail,
            );
            super::maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
            loop_state.history_compact.push(format!(
                "round={} step={} skill={} validation_failed={}",
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_agent_trace(detail)
            ));
            mark_successful_fingerprint = false;
            if loop_state.execution_recipe.is_active() {
                stop_signal = Some(
                    crate::execution_recipe::stop_signal_for_validation_failure(
                        &loop_state.execution_recipe,
                    )
                    .to_string(),
                );
            }
        }
        crate::execution_recipe::ValidationObservation::Inconclusive => {
            record_latest_validation_result(
                loop_state,
                normalized_skill,
                global_step,
                step_in_round,
                "inconclusive",
                "validation_inconclusive",
                action_effect,
            );
            crate::execution_recipe::apply_action_effect_failure(
                &mut loop_state.execution_recipe,
                action_effect,
            );
            register_failed_step_output(
                loop_state,
                global_step,
                step_in_round,
                &format!("skill.{normalized_skill}"),
                &format!("skill({normalized_skill})"),
                "validation result was inconclusive",
            );
            super::maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
            if action_effect.validates {
                loop_state.history_compact.push(format!(
                    "round={} step={} skill={} validation_inconclusive",
                    loop_state.round_no, step_in_round, normalized_skill
                ));
                mark_successful_fingerprint = false;
                if loop_state.execution_recipe.is_active() {
                    stop_signal = Some(
                        crate::execution_recipe::stop_signal_for_validation_failure(
                            &loop_state.execution_recipe,
                        )
                        .to_string(),
                    );
                }
            } else {
                crate::execution_recipe::apply_action_effect_success(
                    &mut loop_state.execution_recipe,
                    crate::execution_recipe::ActionEffect {
                        observes: action_effect.observes,
                        mutates: action_effect.mutates,
                        validates: false,
                    },
                );
            }
        }
    }
    let (ledger_status, ledger_error_kind, ledger_reason) = match &validation_observation {
        crate::execution_recipe::ValidationObservation::Passed => (
            crate::executor::StepExecutionStatus::Ok,
            None,
            "completed_with_observation",
        ),
        crate::execution_recipe::ValidationObservation::Failed(detail) => (
            crate::executor::StepExecutionStatus::Error,
            Some("validation_failed"),
            detail.as_str(),
        ),
        crate::execution_recipe::ValidationObservation::Inconclusive if action_effect.validates => {
            (
                crate::executor::StepExecutionStatus::Error,
                Some("validation_inconclusive"),
                "validation result was inconclusive",
            )
        }
        crate::execution_recipe::ValidationObservation::Inconclusive => (
            crate::executor::StepExecutionStatus::Ok,
            None,
            "completed_with_inconclusive_non_validation_observation",
        ),
    };
    super::attempt_ledger::record_attempt(
        loop_state,
        normalized_skill,
        args_summary,
        ledger_status,
        out,
        ledger_error_kind,
        ledger_reason,
    );
    let had_observed_output = !out.trim().is_empty();
    if had_observed_output {
        loop_state.has_tool_or_skill_output = true;
        let hint = if args_summary.is_empty() {
            super::encode_progress_i18n(
                "telegram.progress.skill_completed",
                &[("skill", normalized_skill)],
            )
        } else {
            super::encode_progress_i18n(
                "telegram.progress.skill_completed_with_args",
                &[("skill", normalized_skill), ("args_summary", args_summary)],
            )
        };
        super::append_progress_hint(state, task, &mut loop_state.progress_messages, hint);
    }
    register_step_output(
        loop_state,
        global_step,
        step_in_round,
        &format!("skill.{normalized_skill}"),
        out,
    );
    if mark_successful_fingerprint {
        *loop_state
            .successful_action_fingerprints
            .entry(fingerprint.to_string())
            .or_insert(0) += 1;
    }
    if matches!(ledger_status, crate::executor::StepExecutionStatus::Ok)
        && had_observed_output
        && skill_extra_requests_user_input(structured_extra)
    {
        loop_state.pending_user_input_required = true;
        loop_state.last_user_visible_respond = Some(out.to_string());
        super::append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            out.to_string(),
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} requires_user_input",
            loop_state.round_no, step_in_round, normalized_skill
        ));
        stop_signal = Some("skill_requires_user_input".to_string());
    }
    info!(
        "executor_result_ok task_id={} round={} step={} type={} output={} trace_only=raw_not_delivery",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        crate::truncate_for_log(out)
    );
    loop_state.history_compact.push(format!(
        "round={} step={} skill={} ok",
        loop_state.round_no, step_in_round, normalized_skill
    ));
    debug!(
        "step_execution_result step_id={} skill={} status={} started_at={} finished_at={}",
        step_execution.step_id,
        step_execution.skill,
        step_execution.status.as_str(),
        step_execution.started_at,
        step_execution.finished_at
    );
    let isolation_artifact_refs =
        capability_isolation_artifact_refs(state, &task.task_id, normalized_skill, action_args);
    let evidence_output = matrix_admitted_external_evidence_output(
        state,
        normalized_skill,
        action_args,
        out,
        structured_extra,
    )
    .or_else(|| structured_extra_evidence_output(out, structured_extra));
    let evidence_output =
        merge_isolation_artifact_refs(evidence_output, out, &isolation_artifact_refs);
    let journal_step_execution = evidence_output
        .map(|evidence_output| crate::executor::StepExecutionResult {
            output: Some(evidence_output),
            ..step_execution.clone()
        })
        .unwrap_or_else(|| step_execution.clone());
    loop_state
        .executed_step_results
        .push(journal_step_execution.clone());
    log_step_journal_summary(
        task,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        loop_state
            .execution_recipe
            .is_active()
            .then(|| loop_state.execution_recipe.phase_summary_line())
            .as_deref(),
        &journal_step_execution,
    );
    if stop_signal.is_none() {
        stop_signal = super::async_start_checkpoint::publish_pending_async_job_start_checkpoint(
            state,
            task,
            loop_state,
            normalized_skill,
            global_step,
            step_in_round,
            structured_extra,
        )?;
    }
    // Raw skill output stays trace/evidence unless the skill explicitly marks it as a user-input prompt.
    let ended_with_user_visible_output =
        stop_signal.as_deref() == Some("skill_requires_user_input");
    Ok(SkillActionOutcome {
        ended_with_user_visible_output,
        stop_signal,
        continue_in_round: false,
    })
}

fn record_latest_validation_result(
    loop_state: &mut LoopState,
    normalized_skill: &str,
    global_step: usize,
    step_in_round: usize,
    status: &'static str,
    status_code: &'static str,
    action_effect: crate::execution_recipe::ActionEffect,
) {
    if !loop_state.execution_recipe.is_active() || !action_effect.validates {
        return;
    }
    loop_state.latest_validation_result = Some(json!({
        "schema_version": 1,
        "source": "agent_loop_step_validation",
        "status": status,
        "status_code": status_code,
        "skill": normalized_skill,
        "global_step": global_step,
        "step_in_round": step_in_round,
    }));
}

async fn handle_skill_step_failure(
    state: &AppState,
    task: &ClaimedTask,
    step_execution: &crate::executor::StepExecutionResult,
    actions: &[crate::AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    idx: usize,
    global_step: usize,
    step_in_round: usize,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    normalized_skill: &str,
    recovery_args: Option<&Value>,
    err: &str,
    action_trace_kind: &str,
) -> Result<Option<String>, String> {
    let error_observation = skill_error_observation_or_raw(normalized_skill, err);
    let progress_error = skill_error_progress_token(normalized_skill, err);
    let ledger_args_summary = recovery_args
        .map(|args| {
            super::build_safe_skill_args_summary(args, super::PROGRESS_ARGS_SUMMARY_MAX_LEN)
        })
        .unwrap_or_default();
    super::attempt_ledger::record_attempt(
        loop_state,
        normalized_skill,
        &ledger_args_summary,
        crate::executor::StepExecutionStatus::Error,
        "",
        None,
        &error_observation,
    );
    let effect = recovery_args
        .map(|args| {
            crate::execution_recipe::apply_target_scope_progress(
                &mut loop_state.execution_recipe,
                state,
                normalized_skill,
                args,
                false,
            );
            crate::execution_recipe::classify_skill_action_effect(state, normalized_skill, args)
        })
        .unwrap_or_default();
    crate::execution_recipe::apply_action_effect_failure(&mut loop_state.execution_recipe, effect);
    super::maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        &format!("skill({normalized_skill})"),
        false,
        &error_observation,
    );
    info!(
        "executor_result_error task_id={} round={} step={} type={} error={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        crate::truncate_for_log(&error_observation)
    );
    loop_state
        .executed_step_results
        .push(step_execution.clone());
    log_step_journal_summary(
        task,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        loop_state
            .execution_recipe
            .is_active()
            .then(|| loop_state.execution_recipe.phase_summary_line())
            .as_deref(),
        step_execution,
    );
    if let Some(policy_block) = crate::skills::parse_policy_block_error(err) {
        register_failed_step_output(
            loop_state,
            global_step,
            step_in_round,
            &format!("skill.{normalized_skill}"),
            &format!("skill({normalized_skill})"),
            &error_observation,
        );
        let message =
            compose_policy_block_delivery(state, task, goal, user_text, &policy_block).await;
        super::append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, message);
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} policy_block reason={}",
            loop_state.round_no, step_in_round, normalized_skill, policy_block.reason_code
        ));
        return Ok(Some("policy_block_user_visible".to_string()));
    }
    if let Some(stop_reason) = try_auto_sudo_retry_after_permission_denied(
        state,
        task,
        loop_state,
        global_step,
        step_in_round,
        goal,
        user_text,
        normalized_skill,
        recovery_args,
        err,
    )
    .await?
    {
        return Ok(stop_reason);
    }
    if let Some(stop_reason) = classify_skill_failure_recovery(
        state,
        actions,
        idx,
        policy.max_steps,
        normalized_skill,
        recovery_args,
        err,
    ) {
        register_failed_step_output(
            loop_state,
            global_step,
            step_in_round,
            &format!("skill.{normalized_skill}"),
            &format!("skill({normalized_skill})"),
            &error_observation,
        );
        register_failed_step_structured_error_fields(
            loop_state,
            &format!("skill.{normalized_skill}"),
            err,
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} failed error={}",
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_agent_trace(&error_observation)
        ));
        publish_failure_recovery_progress(
            state,
            task,
            loop_state,
            step_in_round,
            normalized_skill,
            &progress_error,
            stop_reason,
        );
        return Ok(Some(stop_reason.to_string()));
    }
    let resume_err = build_resume_context_error(
        state,
        task,
        actions,
        round_steps,
        user_text,
        goal,
        &loop_state.subtask_results,
        &loop_state.delivery_messages,
        step_in_round,
        &format!("skill({normalized_skill})"),
        &error_observation,
        Some(err),
    )
    .await;
    Err(resume_err)
}

async fn compose_policy_block_delivery(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy_block: &crate::skills::PolicyBlockError,
) -> String {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let default_text =
        crate::skills::policy_block_default_text(state, task, user_text, policy_block);
    let mut policy_boundary = policy_block.policy_boundary.clone();
    policy_boundary.push("blocked_action_execution_claim_allowed=false".to_string());
    policy_boundary.push("expose_raw_policy_payload=false".to_string());
    policy_boundary.push("expose_internal_action_names=false".to_string());
    let contract = crate::fallback::UserResponseContract::policy_block(
        &policy_block.reason_code,
        user_text,
        goal,
        policy_block.observed_facts.clone(),
        policy_boundary,
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::PolicyBlock,
        &default_text,
    )
    .await
}

pub(super) async fn execute_prepared_skill_action(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[crate::AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    action: &crate::AgentAction,
    fingerprint: &str,
    global_step: usize,
    step_in_round: usize,
    normalized_skill: &str,
    exec_args: Value,
    recovery_args: Option<Value>,
    write_file_effective_path: Option<WriteFileEffectivePath>,
    read_file_requested_path: Option<String>,
    args_summary: String,
    action_trace_kind: &str,
) -> Result<SkillActionOutcome, String> {
    let classification_args = recovery_args.as_ref().unwrap_or(&exec_args);
    if normalized_skill == "subagent" {
        record_subagent_hook_stage(
            state,
            task,
            loop_state,
            crate::agent_hooks::HookStage::SubagentStart,
            &exec_args,
            global_step,
            step_in_round,
            "started",
        )
        .await;
        let subagent_config = super::subagent_runtime::load_subagent_runtime_config(state);
        if super::subagent_runtime::persistent_child_task_requested(&exec_args) {
            let persistent_outcome =
                super::subagent_runtime::record_persistent_child_task_from_args(
                    state,
                    task,
                    loop_state,
                    global_step,
                    step_in_round,
                    &exec_args,
                    &subagent_config,
                );
            let (stop_signal, step_error_signal) = match persistent_outcome {
                Ok(signal) => (Some(signal), None),
                Err(signal) => (Some(signal), Some(signal)),
            };
            record_subagent_step_execution(
                task,
                loop_state,
                global_step,
                step_in_round,
                action_trace_kind,
                step_error_signal,
            );
            record_subagent_hook_stage(
                state,
                task,
                loop_state,
                crate::agent_hooks::HookStage::SubagentStop,
                &exec_args,
                global_step,
                step_in_round,
                if step_error_signal.is_some() {
                    "error"
                } else {
                    "ok"
                },
            )
            .await;
            return Ok(SkillActionOutcome {
                ended_with_user_visible_output: false,
                stop_signal: stop_signal.map(str::to_string),
                continue_in_round: false,
            });
        }
        let stop_signal = super::subagent_runtime::record_subagent_action_from_args_with_config(
            loop_state,
            global_step,
            step_in_round,
            &exec_args,
            &subagent_config,
        )
        .map(str::to_string);
        if stop_signal.is_none() {
            super::subagent_runtime::maybe_run_model_assisted_subagent(
                state,
                task,
                loop_state,
                global_step,
                step_in_round,
                &exec_args,
            )
            .await;
        }
        record_subagent_step_execution(
            task,
            loop_state,
            global_step,
            step_in_round,
            action_trace_kind,
            stop_signal.as_deref(),
        );
        record_subagent_hook_stage(
            state,
            task,
            loop_state,
            crate::agent_hooks::HookStage::SubagentStop,
            &exec_args,
            global_step,
            step_in_round,
            if stop_signal.is_some() { "error" } else { "ok" },
        )
        .await;
        return Ok(SkillActionOutcome {
            ended_with_user_visible_output: false,
            stop_signal,
            continue_in_round: false,
        });
    }
    if let Some(err) =
        capability_isolation_policy_error(state, normalized_skill, classification_args)
    {
        return Ok(handle_preflight_argument_failure(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            normalized_skill,
            classification_args,
            &err,
            action_trace_kind,
        ));
    }
    if let Some(err) = evidence_policy_action_policy_error(
        state,
        loop_state,
        normalized_skill,
        classification_args,
        action_trace_kind,
    ) {
        return Ok(handle_preflight_argument_failure(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            normalized_skill,
            classification_args,
            &err,
            action_trace_kind,
        ));
    }
    if let Some(err) = unresolved_runtime_template_argument_error(
        normalized_skill,
        &exec_args,
        classification_args,
    ) {
        return Ok(handle_preflight_argument_failure(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            normalized_skill,
            classification_args,
            &err,
            action_trace_kind,
        ));
    }
    if let Some(err) = structured_observation_path_argument_error(normalized_skill, &exec_args) {
        return Ok(handle_preflight_argument_failure(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            normalized_skill,
            classification_args,
            &err,
            action_trace_kind,
        ));
    }
    let pre_tool_use_evaluation = crate::agent_hooks::pre_tool_use_outcome_for_state(
        state,
        &task.task_id,
        normalized_skill,
        &exec_args,
    )
    .await;
    record_hook_evaluation_observation(
        loop_state,
        normalized_skill,
        global_step,
        step_in_round,
        &pre_tool_use_evaluation,
    );
    if pre_tool_use_evaluation.requires_confirmation() {
        let permission_evaluation = record_permission_request_hook(
            state,
            task,
            loop_state,
            normalized_skill,
            &pre_tool_use_evaluation.outcome.action_ref,
            global_step,
            step_in_round,
        )
        .await;
        if permission_evaluation.requires_background_wait() {
            super::support::publish_agent_loop_checkpoint_progress(
                state,
                task,
                loop_state,
                "hook_background_wait",
            );
            return Ok(SkillActionOutcome {
                ended_with_user_visible_output: false,
                stop_signal: Some("hook_background_wait".to_string()),
                continue_in_round: false,
            });
        }
        if permission_evaluation.outcome.decision_kind()
            == Some(crate::policy_decision::PolicyDecision::Deny)
        {
            if let Some(err) =
                crate::agent_hooks::structured_error_for_outcome(&permission_evaluation.outcome)
            {
                return Ok(handle_preflight_argument_failure(
                    state,
                    task,
                    loop_state,
                    global_step,
                    step_in_round,
                    normalized_skill,
                    classification_args,
                    &err,
                    action_trace_kind,
                ));
            }
        }
        super::publish_agent_loop_user_input_checkpoint_progress(
            state,
            task,
            loop_state,
            "hook_confirmation_required",
            normalized_skill,
            &pre_tool_use_evaluation.outcome.action_ref,
            &exec_args,
        );
        return Ok(SkillActionOutcome {
            ended_with_user_visible_output: false,
            stop_signal: Some("hook_confirmation_required".to_string()),
            continue_in_round: false,
        });
    }
    if pre_tool_use_evaluation.requires_background_wait() {
        super::support::publish_agent_loop_checkpoint_progress(
            state,
            task,
            loop_state,
            "hook_background_wait",
        );
        return Ok(SkillActionOutcome {
            ended_with_user_visible_output: false,
            stop_signal: Some("hook_background_wait".to_string()),
            continue_in_round: false,
        });
    }
    if let Some(err) =
        crate::agent_hooks::structured_error_for_outcome(&pre_tool_use_evaluation.outcome)
    {
        return Ok(handle_preflight_argument_failure(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            normalized_skill,
            classification_args,
            &err,
            action_trace_kind,
        ));
    }
    info!(
        "{} executor_step_execute task_id={} round={} step={} type={} skill={} args={}",
        crate::highlight_tag("skill"),
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        normalized_skill,
        crate::truncate_for_log(&exec_args.to_string())
    );
    let structured_validation = Arc::new(Mutex::new(None::<Value>));
    let structured_extra = Arc::new(Mutex::new(None::<Value>));
    let structured_validation_slot = Arc::clone(&structured_validation);
    let structured_extra_slot = Arc::clone(&structured_extra);
    let exec_args_for_run = exec_args.clone();
    let mcp_descriptor = state.mcp_tool(normalized_skill);
    let is_mcp_tool = mcp_descriptor.is_some();
    let mcp_started_at = is_mcp_tool.then(Instant::now);
    let step_execution =
        crate::executor::execute_step(&format!("step_{global_step}"), action, || {
            let structured_validation_slot = Arc::clone(&structured_validation_slot);
            let structured_extra_slot = Arc::clone(&structured_extra_slot);
            let exec_args_for_run = exec_args_for_run.clone();
            async move {
                if is_mcp_tool {
                    let (raw, extra) =
                        run_mcp_tool_observation(state, task, normalized_skill, exec_args_for_run)
                            .await?;
                    if let Ok(mut slot) = structured_extra_slot.lock() {
                        *slot = Some(extra);
                    }
                    return Ok(raw);
                }
                let outcome =
                    run_skill_with_runner_outcome(state, task, normalized_skill, exec_args_for_run)
                        .await?;
                if let Ok(mut slot) = structured_validation_slot.lock() {
                    *slot = outcome.validation.clone();
                }
                if let Ok(mut slot) = structured_extra_slot.lock() {
                    *slot = outcome.extra.clone();
                }
                Ok(outcome.text)
            }
        })
        .await;
    let structured_validation = structured_validation
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
    let structured_extra = structured_extra.lock().ok().and_then(|slot| slot.clone());
    if let (Some(descriptor), Some(started_at)) = (mcp_descriptor.as_ref(), mcp_started_at) {
        record_mcp_tool_execution_observation(
            state,
            task,
            loop_state,
            descriptor,
            &step_execution,
            structured_extra.as_ref(),
            started_at.elapsed(),
        );
    }
    record_post_tool_use_hook_observations(
        state,
        task,
        loop_state,
        normalized_skill,
        &exec_args,
        global_step,
        step_in_round,
        step_execution.status,
    )
    .await;
    let raw_action_effect = crate::execution_recipe::classify_skill_action_effect(
        state,
        normalized_skill,
        classification_args,
    );
    let action_effect = crate::execution_recipe::effective_action_effect_for_recipe(
        loop_state.execution_recipe,
        raw_action_effect,
    );
    let validation_observation = if raw_action_effect.validates {
        crate::execution_recipe::assess_validation_output_with_structured(
            state,
            normalized_skill,
            classification_args,
            step_execution.output.as_deref().unwrap_or_default(),
            structured_validation.as_ref(),
        )
    } else {
        crate::execution_recipe::ValidationObservation::Passed
    };
    match step_execution.output.as_ref() {
        Some(out) => {
            let outcome = handle_skill_step_success(
                state,
                task,
                loop_state,
                fingerprint,
                &step_execution,
                global_step,
                step_in_round,
                normalized_skill,
                action_trace_kind,
                &args_summary,
                &exec_args,
                out,
                action_effect,
                validation_observation,
                structured_extra.as_ref(),
                write_file_effective_path.as_ref(),
                read_file_requested_path.as_deref(),
            )
            .await?;
            Ok(outcome)
        }
        None => {
            if !repo::is_task_still_running(state, &task.task_id).unwrap_or(true) {
                return Err(TASK_CANCELED_ERR.to_string());
            }
            let err = step_execution.error.clone().unwrap_or_default();
            match handle_skill_step_failure(
                state,
                task,
                &step_execution,
                actions,
                round_steps,
                loop_state,
                idx,
                global_step,
                step_in_round,
                goal,
                user_text,
                policy,
                normalized_skill,
                recovery_args.as_ref().or(Some(&exec_args)),
                &err,
                action_trace_kind,
            )
            .await?
            {
                Some(stop_reason) if stop_reason == "recoverable_failure_continue_in_round" => {
                    Ok(SkillActionOutcome {
                        ended_with_user_visible_output: false,
                        stop_signal: None,
                        continue_in_round: true,
                    })
                }
                Some(stop_reason) => Ok(SkillActionOutcome {
                    ended_with_user_visible_output: false,
                    stop_signal: Some(stop_reason),
                    continue_in_round: false,
                }),
                None => Ok(SkillActionOutcome {
                    ended_with_user_visible_output: false,
                    stop_signal: None,
                    continue_in_round: false,
                }),
            }
        }
    }
}

#[cfg(test)]
#[path = "skill_execution_async_start_tests.rs"]
mod async_start_tests;
#[cfg(test)]
#[path = "skill_execution_hook_policy_tests.rs"]
mod hook_policy_tests;
#[cfg(test)]
#[path = "skill_execution_isolation_tests.rs"]
mod isolation_tests;
#[cfg(test)]
#[path = "skill_execution_mcp_tests.rs"]
mod mcp_tests;
#[cfg(test)]
#[path = "skill_execution_permission_tests.rs"]
mod permission_tests;
#[cfg(test)]
#[path = "skill_execution_preflight_dry_run_tests.rs"]
mod preflight_dry_run_tests;
#[cfg(test)]
#[path = "skill_execution_tests.rs"]
pub(super) mod tests;

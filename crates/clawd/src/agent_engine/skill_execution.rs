use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::{
    build_resume_context_error, classify_skill_failure_recovery, ensure_task_running,
    register_failed_step_output, register_file_path_output, register_step_output,
    remember_written_file_alias, AgentLoopGuardPolicy, AppState, ClaimedTask, LoopState,
    SkillActionOutcome, WriteFileEffectivePath, TASK_CANCELED_ERR,
};
use crate::{repo, run_skill_with_runner_outcome};

fn log_step_journal_summary(
    task: &ClaimedTask,
    round_no: usize,
    step_in_round: usize,
    action_trace_kind: &str,
    step_execution: &crate::executor::StepExecutionResult,
) {
    let mut journal =
        crate::task_journal::TaskJournal::new(format!("step:{}", step_execution.skill));
    journal.record_context_bundle_summary(format!(
        "round={} step={} action_type={}",
        round_no, step_in_round, action_trace_kind
    ));
    journal.push_step_result(step_execution);
    info!(
        "task_journal_summary task_id={} kind=ask phase=step_execute round={} step={} {}",
        task.task_id,
        round_no,
        step_in_round,
        journal.to_log_json()
    );
}

fn matches_json_schema_type(value: &Value, expected_type: &str) -> bool {
    match expected_type {
        "string" => value.is_string(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        _ => true,
    }
}

fn validate_json_contract(value: &Value, schema: &Value) -> Result<(), String> {
    let expected_type = schema.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if !expected_type.is_empty() && !matches_json_schema_type(value, expected_type) {
        return Err(format!("expected type `{expected_type}`"));
    }
    if expected_type == "object" {
        let obj = value
            .as_object()
            .ok_or_else(|| "expected object output".to_string())?;
        if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
            for key in required.iter().filter_map(|item| item.as_str()) {
                if !obj.contains_key(key) {
                    return Err(format!("missing required field `{key}`"));
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
            for (key, prop_schema) in properties {
                let Some(field_value) = obj.get(key) else {
                    continue;
                };
                if let Some(field_type) = prop_schema.get("type").and_then(|v| v.as_str()) {
                    if !matches_json_schema_type(field_value, field_type) {
                        return Err(format!("field `{key}` expected type `{field_type}`"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_skill_output_contract(
    state: &AppState,
    normalized_skill: &str,
    output: &str,
) -> Result<(), String> {
    let Some((output_kind, schema)) = state.skill_output_contract(normalized_skill) else {
        return Ok(());
    };
    let candidate = if output_kind == claw_core::skill_registry::OutputKind::Text {
        if schema.get("type").and_then(|v| v.as_str()) == Some("object")
            && schema
                .get("properties")
                .and_then(|v| v.as_object())
                .map(|props| props.contains_key("text"))
                .unwrap_or(false)
        {
            json!({ "text": output })
        } else {
            Value::String(output.to_string())
        }
    } else {
        crate::parse_llm_json_raw_or_any::<Value>(output)
            .unwrap_or_else(|| Value::String(output.to_string()))
    };
    validate_json_contract(&candidate, &schema)
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
    out: &str,
    write_file_effective_path: Option<&WriteFileEffectivePath>,
    read_file_requested_path: Option<&str>,
    cache_publishable_chat_output: bool,
) -> Result<bool, String> {
    ensure_task_running(state, task)?;
    let mut publishable_chat_output = false;
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
    if cache_publishable_chat_output
        && normalized_skill == "chat"
        && crate::semantic_judge::is_publishable_raw(state, task, out).await
    {
        loop_state.last_publishable_chat_output = Some(out.to_string());
        publishable_chat_output = true;
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
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        &format!("skill({normalized_skill})"),
        true,
        out,
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
    *loop_state
        .successful_action_fingerprints
        .entry(fingerprint.to_string())
        .or_insert(0) += 1;
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
    loop_state
        .executed_step_results
        .push(step_execution.clone());
    log_step_journal_summary(
        task,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        step_execution,
    );
    // Raw skill output is trace/evidence, not final user-visible delivery.
    // Only publishable chat output counts as terminal user-visible output here.
    Ok(publishable_chat_output)
}

fn handle_skill_step_failure(
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
    let user_visible_err = crate::skills::normalize_skill_error_for_user(normalized_skill, err);
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        &format!("skill({normalized_skill})"),
        false,
        &user_visible_err,
    );
    info!(
        "executor_result_error task_id={} round={} step={} type={} error={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        crate::truncate_for_log(&user_visible_err)
    );
    loop_state
        .executed_step_results
        .push(step_execution.clone());
    log_step_journal_summary(
        task,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        step_execution,
    );
    let has_remaining_actions = actions
        .iter()
        .take(policy.max_steps.max(1))
        .skip(idx + 1)
        .any(|action| !matches!(action, crate::AgentAction::Think { .. }));
    if normalized_skill.eq_ignore_ascii_case("chat")
        && loop_state.has_tool_or_skill_output
        && loop_state.delivery_messages.is_empty()
        && !has_remaining_actions
    {
        register_failed_step_output(
            loop_state,
            global_step,
            step_in_round,
            &format!("skill.{normalized_skill}"),
            &format!("skill({normalized_skill})"),
            &user_visible_err,
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} failed error={} finalize_from_observed=true",
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_agent_trace(&user_visible_err)
        ));
        return Ok(Some("recoverable_failure_finalize".to_string()));
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
            &user_visible_err,
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} failed error={}",
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_agent_trace(&user_visible_err)
        ));
        return Ok(Some(stop_reason.to_string()));
    }
    let resume_err = build_resume_context_error(
        state,
        actions,
        round_steps,
        user_text,
        goal,
        &loop_state.subtask_results,
        &loop_state.delivery_messages,
        step_in_round,
        &format!("skill({normalized_skill})"),
        &user_visible_err,
    );
    Err(resume_err)
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
    cache_publishable_chat_output: bool,
) -> Result<SkillActionOutcome, String> {
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
    let step_execution =
        crate::executor::execute_step(&format!("step_{global_step}"), action, || async {
            run_skill_with_runner_outcome(state, task, normalized_skill, exec_args.clone())
                .await
                .map(|outcome| outcome.text)
        })
        .await;
    match step_execution.output.as_ref() {
        Some(out) => {
            let ended_with_user_visible_output = handle_skill_step_success(
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
                out,
                write_file_effective_path.as_ref(),
                read_file_requested_path.as_deref(),
                cache_publishable_chat_output,
            )
            .await?;
            Ok(SkillActionOutcome {
                ended_with_user_visible_output,
                stop_signal: None,
                continue_in_round: false,
            })
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
            )? {
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

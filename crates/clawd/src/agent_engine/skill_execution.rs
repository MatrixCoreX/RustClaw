use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
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
    execution_recipe_summary: Option<&str>,
    step_execution: &crate::executor::StepExecutionResult,
) {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        &task.task_id,
        "ask",
        format!("step:{}", step_execution.skill),
    );
    let mut summary = format!(
        "round={} step={} action_type={}",
        round_no, step_in_round, action_trace_kind
    );
    if let Some(recipe_summary) = execution_recipe_summary.filter(|v| !v.trim().is_empty()) {
        summary.push(' ');
        summary.push_str(recipe_summary);
    }
    journal.record_context_bundle_summary(summary);
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

fn remember_skill_metadata(loop_state: &mut LoopState, normalized_skill: &str) {
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), normalized_skill.to_string());
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn command_already_requests_sudo(command: &str) -> bool {
    command
        .split_whitespace()
        .any(|part| part == "sudo" || part.ends_with("/sudo"))
}

fn bounded_u64_arg(args: &Value, names: &[&str], default: u64, min: u64, max: u64) -> u64 {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_u64))
        .unwrap_or(default)
        .clamp(min, max)
}

fn sudo_list_dir_command(args: &Value) -> Option<String> {
    let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let max_entries = bounded_u64_arg(args, &["limit", "max_entries"], 200, 1, 1000);
    Some(format!(
        "sudo -n sh -c {} sh {} | sort | head -n {}",
        shell_single_quote(
            r#"dir=$1; for item in "$dir"/* "$dir"/.[!.]* "$dir"/..?*; do [ -e "$item" ] || continue; basename "$item"; done"#
        ),
        shell_single_quote(path),
        max_entries
    ))
}

fn sudo_read_range_command(args: &Value) -> Option<String> {
    let path = args.get("path").and_then(Value::as_str)?;
    let quoted_path = shell_single_quote(path);
    let n = bounded_u64_arg(args, &["n"], 20, 1, 500);
    let has_start_or_end = args.get("start_line").is_some() || args.get("end_line").is_some();
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_else(|| {
            if has_start_or_end {
                "range".to_string()
            } else {
                "head".to_string()
            }
        });
    match mode.as_str() {
        "tail" => Some(format!("sudo -n tail -n {n} {quoted_path}")),
        "range" => {
            let start = args
                .get("start_line")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .max(1);
            let end = args
                .get("end_line")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| start.saturating_add(n).saturating_sub(1))
                .max(start);
            Some(format!(
                "sudo -n sed -n {} {}",
                shell_single_quote(&format!("{start},{end}p")),
                quoted_path
            ))
        }
        _ => Some(format!(
            "sudo -n sed -n {} {}",
            shell_single_quote(&format!("1,{n}p")),
            quoted_path
        )),
    }
}

fn sudo_structured_read_command(normalized_skill: &str, args: &Value) -> Option<String> {
    match normalized_skill {
        "read_file" => args
            .get("path")
            .and_then(Value::as_str)
            .map(|path| format!("sudo -n cat {}", shell_single_quote(path))),
        "list_dir" => sudo_list_dir_command(args),
        "system_basic" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("info")
                .trim()
                .to_ascii_lowercase();
            match action.as_str() {
                "read_range" => sudo_read_range_command(args),
                "inventory_dir" => sudo_list_dir_command(args),
                "count_inventory" => args.get("path").and_then(Value::as_str).map(|path| {
                    format!(
                        "sudo -n sh -c {} sh {}",
                        shell_single_quote(
                            r#"dir=$1; count=0; for item in "$dir"/* "$dir"/.[!.]* "$dir"/..?*; do [ -e "$item" ] || continue; count=$((count + 1)); done; printf '%s\n' "$count""#
                        ),
                        shell_single_quote(path)
                    )
                }),
                "extract_field" | "extract_fields" | "structured_keys" => args
                    .get("path")
                    .and_then(Value::as_str)
                    .map(|path| format!("sudo -n cat {}", shell_single_quote(path))),
                _ => None,
            }
        }
        _ => None,
    }
}

fn build_auto_sudo_retry_args(
    state: &AppState,
    task: &ClaimedTask,
    normalized_skill: &str,
    recovery_args: Option<&Value>,
    err: &str,
) -> Option<Value> {
    if !crate::skills::error_looks_like_os_permission_denied(err)
        || !crate::skills::task_allows_sudo(state, Some(task))
        || !state.get_skills_list().contains("run_cmd")
        || !state.task_allows_skill(task, "run_cmd")
    {
        return None;
    }
    let args = recovery_args?;
    if normalized_skill == "run_cmd" {
        let command = args.get("command").and_then(Value::as_str)?.trim();
        if command.is_empty() || command_already_requests_sudo(command) {
            return None;
        }
        let mut retry_args = args.clone();
        let obj = retry_args.as_object_mut()?;
        obj.remove(super::CLAWD_CONTINUE_ON_ERROR_ARG);
        obj.remove(super::CLAWD_LITERAL_COMMAND_ARG);
        obj.remove(super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG);
        obj.remove(super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG);
        obj.remove(crate::execution_recipe::CLAWD_VALIDATION_ARG);
        obj.insert(
            "command".to_string(),
            Value::String(format!("sudo -n bash -lc {}", shell_single_quote(command))),
        );
        return Some(retry_args);
    }
    let command = sudo_structured_read_command(normalized_skill, args)?;
    Some(json!({
        "command": command,
        "cwd": ".",
        "timeout_seconds": 10,
        "idle_timeout_seconds": 3,
        "max_output_bytes": 32768
    }))
}

fn auto_sudo_retry_failed_delivery(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    err: &str,
) -> String {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let detail = crate::truncate_for_agent_trace(err.trim());
    if language_hint.to_ascii_lowercase().starts_with("en") {
        format!(
            "The first attempt was denied by the operating system, so this admin-authorized task retried with `sudo -n`, but the retry still failed: {detail}. Confirm that the `clawd` process user has passwordless sudo or run the service with the required OS permission."
        )
    } else {
        format!(
            "首次执行被操作系统拒绝后，当前 admin 授权任务已自动用 `sudo -n` 重试，但重试仍失败：{detail}。请确认 `clawd` 进程用户具备免密 sudo，或用具备目标系统权限的服务用户运行。"
        )
    }
}

fn compact_progress_error(err: &str) -> String {
    crate::truncate_for_agent_trace(
        &crate::visible_text::sanitize_user_visible_text(err)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" | "),
    )
}

fn publish_failure_progress(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    step_in_round: usize,
    normalized_skill: &str,
    user_visible_err: &str,
) {
    let step = step_in_round.to_string();
    let error = compact_progress_error(user_visible_err);
    super::append_progress_hint(
        state,
        task,
        &mut loop_state.progress_messages,
        super::encode_progress_i18n(
            "telegram.progress.step_failed",
            &[
                ("step", &step),
                ("skill", normalized_skill),
                ("error", &error),
            ],
        ),
    );
}

fn publish_retry_progress(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    stop_reason: &str,
) {
    let key = match stop_reason {
        "recoverable_failure_continue_in_round" => "telegram.progress.retry_continue",
        "recoverable_failure_continue_round" => "telegram.progress.retry_replan",
        "auto_sudo_retry" => "telegram.progress.retry_sudo",
        _ => return,
    };
    super::append_progress_hint(
        state,
        task,
        &mut loop_state.progress_messages,
        super::encode_progress_i18n(key, &[]),
    );
}

fn publish_failure_recovery_progress(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    step_in_round: usize,
    normalized_skill: &str,
    user_visible_err: &str,
    stop_reason: &str,
) {
    publish_failure_progress(
        state,
        task,
        loop_state,
        step_in_round,
        normalized_skill,
        user_visible_err,
    );
    publish_retry_progress(state, task, loop_state, stop_reason);
}

async fn try_auto_sudo_retry_after_permission_denied(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    goal: &str,
    user_text: &str,
    normalized_skill: &str,
    recovery_args: Option<&Value>,
    err: &str,
) -> Result<Option<Option<String>>, String> {
    let Some(retry_args) =
        build_auto_sudo_retry_args(state, task, normalized_skill, recovery_args, err)
    else {
        return Ok(None);
    };
    let user_visible_err = crate::skills::normalize_skill_error_for_user(normalized_skill, err);
    publish_failure_recovery_progress(
        state,
        task,
        loop_state,
        step_in_round,
        normalized_skill,
        &user_visible_err,
        "auto_sudo_retry",
    );
    let retry_action = crate::AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: retry_args.clone(),
    };
    info!(
        "auto_sudo_retry task_id={} round={} step={} from_skill={} args={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        normalized_skill,
        crate::truncate_for_log(&retry_args.to_string())
    );
    let retry_step = crate::executor::execute_step(
        &format!("step_{global_step}_sudo_retry"),
        &retry_action,
        || async {
            run_skill_with_runner_outcome(state, task, "run_cmd", retry_args.clone())
                .await
                .map(|outcome| outcome.text)
        },
    )
    .await;

    match retry_step.output.as_deref() {
        Some(out) => {
            loop_state.history_compact.push(format!(
                "round={} step={} skill={} permission_denied_auto_sudo_retry ok",
                loop_state.round_no, step_in_round, normalized_skill
            ));
            let raw_effect = crate::execution_recipe::classify_skill_action_effect(
                state,
                "run_cmd",
                &retry_args,
            );
            let action_effect = crate::execution_recipe::effective_action_effect_for_recipe(
                loop_state.execution_recipe,
                raw_effect,
            );
            let args_summary = super::build_safe_skill_args_summary(
                &retry_args,
                super::PROGRESS_ARGS_SUMMARY_MAX_LEN,
            );
            let outcome = handle_skill_step_success(
                state,
                task,
                loop_state,
                &format!("skill:run_cmd:auto_sudo_retry:{}", retry_args),
                &retry_step,
                global_step,
                step_in_round,
                "run_cmd",
                "call_skill(auto_sudo_retry)",
                &args_summary,
                &retry_args,
                out,
                action_effect,
                crate::execution_recipe::ValidationObservation::Passed,
                None,
                None,
            )
            .await?;
            Ok(Some(outcome.stop_signal))
        }
        None => {
            let retry_err = retry_step.error.clone().unwrap_or_default();
            let user_visible_retry_err =
                crate::skills::normalize_skill_error_for_user("run_cmd", &retry_err);
            crate::append_subtask_result(
                &mut loop_state.subtask_results,
                global_step,
                "skill(run_cmd:auto_sudo_retry)",
                false,
                &user_visible_retry_err,
            );
            register_failed_step_output(
                loop_state,
                global_step,
                step_in_round,
                "skill.run_cmd",
                "skill(run_cmd:auto_sudo_retry)",
                &user_visible_retry_err,
            );
            loop_state.executed_step_results.push(retry_step.clone());
            log_step_journal_summary(
                task,
                loop_state.round_no,
                step_in_round,
                "call_skill(auto_sudo_retry)",
                loop_state
                    .execution_recipe
                    .is_active()
                    .then(|| loop_state.execution_recipe.phase_summary_line())
                    .as_deref(),
                &retry_step,
            );
            if let Some(policy_block) = crate::skills::parse_policy_block_error(&retry_err) {
                let message =
                    compose_policy_block_delivery(state, task, goal, user_text, &policy_block)
                        .await;
                super::append_delivery_message(
                    &task.task_id,
                    &mut loop_state.delivery_messages,
                    message,
                );
                return Ok(Some(Some("policy_block_user_visible".to_string())));
            }
            let message =
                auto_sudo_retry_failed_delivery(state, task, user_text, &user_visible_retry_err);
            super::append_delivery_message(
                &task.task_id,
                &mut loop_state.delivery_messages,
                message,
            );
            loop_state.history_compact.push(format!(
                "round={} step={} skill={} permission_denied_auto_sudo_retry failed error={}",
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_agent_trace(&user_visible_retry_err)
            ));
            Ok(Some(Some(
                "auto_sudo_retry_failed_user_visible".to_string(),
            )))
        }
    }
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
            crate::execution_recipe::apply_action_effect_success(
                &mut loop_state.execution_recipe,
                action_effect,
            );
            super::maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
        }
        crate::execution_recipe::ValidationObservation::Failed(detail) => {
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
        loop_state
            .execution_recipe
            .is_active()
            .then(|| loop_state.execution_recipe.phase_summary_line())
            .as_deref(),
        step_execution,
    );
    // Raw skill output is trace/evidence, not final user-visible delivery.
    Ok(SkillActionOutcome {
        ended_with_user_visible_output: false,
        stop_signal,
        continue_in_round: false,
    })
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
    let user_visible_err = crate::skills::normalize_skill_error_for_user(normalized_skill, err);
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
            &user_visible_err,
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
            &user_visible_err,
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} failed error={}",
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_agent_trace(&user_visible_err)
        ));
        publish_failure_recovery_progress(
            state,
            task,
            loop_state,
            step_in_round,
            normalized_skill,
            &user_visible_err,
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
        &user_visible_err,
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
    policy_boundary.push("Do not claim the blocked action was executed.".to_string());
    policy_boundary.push("Do not expose raw policy payloads or internal action names.".to_string());
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
    let structured_validation_slot = Arc::clone(&structured_validation);
    let exec_args_for_run = exec_args.clone();
    let step_execution =
        crate::executor::execute_step(&format!("step_{global_step}"), action, || {
            let structured_validation_slot = Arc::clone(&structured_validation_slot);
            let exec_args_for_run = exec_args_for_run.clone();
            async move {
                let outcome =
                    run_skill_with_runner_outcome(state, task, normalized_skill, exec_args_for_run)
                        .await?;
                if let Ok(mut slot) = structured_validation_slot.lock() {
                    *slot = outcome.validation.clone();
                }
                Ok(outcome.text)
            }
        })
        .await;
    let structured_validation = structured_validation
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
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
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, RwLock};

    use super::{
        build_auto_sudo_retry_args, handle_skill_step_failure, handle_skill_step_success,
        AgentLoopGuardPolicy, LoopState,
    };
    use crate::{
        AgentRuntimeConfig, AppState, ClaimedTask, SkillViewsSnapshot, ToolsPolicy,
        DEFAULT_AGENT_ID,
    };
    use claw_core::config::{AgentConfig, ToolsConfig};
    use rusqlite::params;

    fn test_state() -> AppState {
        let db_pool = crate::db_init::test_pool();
        {
            let db = db_pool.get().expect("get db conn");
            db.execute_batch(
                r#"
                CREATE TABLE tasks (
                    task_id TEXT PRIMARY KEY,
                    status TEXT NOT NULL,
                    result_json TEXT,
                    updated_at INTEGER
                );
                INSERT INTO tasks (task_id, status, result_json, updated_at)
                VALUES ('task-skill-exec', 'running', NULL, 0);
                "#,
            )
            .expect("seed tasks");
        }
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                db: db_pool,
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: None,
                    skills_list: Arc::new(HashSet::new()),
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                locator_scan_max_depth: 3,
                locator_scan_max_files: 200,
                tools_policy: Arc::new(
                    ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
                ),
                ..crate::SkillRuntime::test_default()
            },
            policy: crate::PolicyConfig::test_default(),
            worker: crate::WorkerConfig::test_default(),
            metrics: crate::TaskMetricsRegistry::default(),
            channels: crate::ChannelConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
            ask_states: crate::AskStateRegistry::default(),
        }
    }

    fn test_task() -> ClaimedTask {
        ClaimedTask {
            task_id: "task-skill-exec".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "telegram".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    fn insert_auth_key(state: &AppState, user_key: &str, role: &str) {
        let db = state.core.db.get().expect("db pool");
        db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
            .expect("create auth schema");
        db.execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             VALUES (?1, ?2, 1, '123', NULL)",
            params![user_key, role],
        )
        .expect("insert auth key");
    }

    fn enable_test_skills(state: &AppState, skills: &[&str]) {
        let set = skills
            .iter()
            .map(|skill| skill.to_string())
            .collect::<HashSet<_>>();
        *state
            .core
            .skill_views_snapshot
            .write()
            .expect("write skill snapshot") = Arc::new(SkillViewsSnapshot {
            registry: None,
            skills_list: Arc::new(set),
        });
    }

    fn admin_task() -> ClaimedTask {
        let mut task = test_task();
        task.user_key = Some("rk-admin".to_string());
        task
    }

    fn test_policy() -> AgentLoopGuardPolicy {
        AgentLoopGuardPolicy {
            max_steps: 16,
            max_rounds: 2,
            recoverable_failure_extra_rounds: 1,
            repeat_action_limit: 4,
            no_progress_limit: 1,
            multi_round_enabled: true,
            ops_closed_loop: Default::default(),
        }
    }

    #[test]
    fn auto_sudo_retry_builds_structured_read_range_retry_for_admin_permission_denied() {
        let mut state = test_state();
        state.policy.allow_sudo = true;
        enable_test_skills(&state, &["run_cmd", "system_basic"]);
        insert_auth_key(&state, "rk-admin", "admin");
        let task = admin_task();

        let retry = build_auto_sudo_retry_args(
            &state,
            &task,
            "system_basic",
            Some(&serde_json::json!({
                "action": "read_range",
                "path": "/etc/shadow",
                "n": 1
            })),
            &crate::skills::structured_skill_error_from_parts(
                "system_basic",
                "permission_denied",
                "read_range failed for /etc/shadow",
                Some("linux"),
                Some(serde_json::json!({
                    "operation": "metadata",
                    "path": "/etc/shadow"
                })),
            ),
        )
        .expect("admin permission denial should trigger sudo retry");

        let command = retry
            .get("command")
            .and_then(|value| value.as_str())
            .expect("retry command");
        assert!(command.starts_with("sudo -n "), "got: {command}");
        assert!(command.contains("/etc/shadow"), "got: {command}");
        assert!(command.contains("sed"), "got: {command}");
        assert!(!command.contains(" -- "), "got: {command}");
        assert!(!command.contains("-printf"), "got: {command}");
    }

    #[test]
    fn auto_sudo_retry_uses_posix_directory_listing_for_cross_platform_hosts() {
        let mut state = test_state();
        state.policy.allow_sudo = true;
        enable_test_skills(&state, &["run_cmd", "system_basic"]);
        insert_auth_key(&state, "rk-admin", "admin");
        let task = admin_task();

        let retry = build_auto_sudo_retry_args(
            &state,
            &task,
            "system_basic",
            Some(&serde_json::json!({
                "action": "inventory_dir",
                "path": "/var/log",
                "max_entries": 5
            })),
            &crate::skills::structured_skill_error_from_parts(
                "system_basic",
                "permission_denied",
                "read_dir failed for /var/log",
                Some("linux"),
                Some(serde_json::json!({
                    "operation": "read_dir",
                    "path": "/var/log"
                })),
            ),
        )
        .expect("admin permission denial should trigger sudo retry");

        let command = retry
            .get("command")
            .and_then(|value| value.as_str())
            .expect("retry command");
        assert!(command.starts_with("sudo -n sh -c "), "got: {command}");
        assert!(command.contains("basename"), "got: {command}");
        assert!(command.contains("/var/log"), "got: {command}");
        assert!(!command.contains("-printf"), "got: {command}");
        assert!(!command.contains("-maxdepth"), "got: {command}");
    }

    #[test]
    fn auto_sudo_retry_does_not_trigger_for_non_admin_or_existing_sudo() {
        let mut state = test_state();
        state.policy.allow_sudo = true;
        enable_test_skills(&state, &["run_cmd"]);
        insert_auth_key(&state, "rk-user", "user");
        let mut user_task = test_task();
        user_task.user_key = Some("rk-user".to_string());
        let err = "Command failed with exit code 1\nstderr:\nPermission denied";

        assert!(build_auto_sudo_retry_args(
            &state,
            &user_task,
            "run_cmd",
            Some(&serde_json::json!({"command": "cat /root/secret"})),
            err,
        )
        .is_none());

        insert_auth_key(&state, "rk-admin", "admin");
        let task = admin_task();
        assert!(build_auto_sudo_retry_args(
            &state,
            &task,
            "run_cmd",
            Some(&serde_json::json!({"command": "sudo cat /root/secret"})),
            err,
        )
        .is_none());
    }

    fn ok_step(step_id: &str, skill: &str, output: &str) -> crate::executor::StepExecutionResult {
        crate::executor::StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        }
    }

    fn failed_step(
        step_id: &str,
        skill: &str,
        error: &str,
    ) -> crate::executor::StepExecutionResult {
        crate::executor::StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some(error.to_string()),
            started_at: 0,
            finished_at: 0,
        }
    }

    #[tokio::test]
    async fn policy_block_failure_appends_user_visible_delivery() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        let err = crate::skills::policy_block_error(
            "path_outside_workspace",
            vec!["denied_path: /etc/shadow".to_string()],
            vec!["Do not access paths outside workspace.".to_string()],
        );
        let step = failed_step("step_1", "read_file", &err);

        let stop = handle_skill_step_failure(
            &state,
            &task,
            &step,
            &[],
            &["skill(read_file)".to_string()],
            &mut loop_state,
            0,
            1,
            1,
            "Read the first line of /etc/shadow.",
            "Read the first line of /etc/shadow",
            &test_policy(),
            "read_file",
            Some(&serde_json::json!({"path": "/etc/shadow"})),
            &err,
            "skill",
        )
        .await
        .expect("policy block should be converted to delivery");

        assert_eq!(stop.as_deref(), Some("policy_block_user_visible"));
        assert_eq!(loop_state.delivery_messages.len(), 1);
        assert!(loop_state.delivery_messages[0].contains("/etc/shadow"));
        assert!(loop_state.delivery_messages[0].contains("workspace"));
        assert!(loop_state
            .output_vars
            .get("failed_step.error")
            .is_some_and(|value| value.contains("path_outside_workspace")));
    }

    #[tokio::test]
    async fn non_recoverable_failure_preserves_resume_context_and_user_error() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        let err = "planner schema mismatch: missing field `path`";
        let step = failed_step("step_1", "fragile_skill", err);
        let actions = vec![
            crate::AgentAction::CallSkill {
                skill: "fragile_skill".to_string(),
                args: serde_json::json!({}),
            },
            crate::AgentAction::CallSkill {
                skill: "next_skill".to_string(),
                args: serde_json::json!({}),
            },
        ];

        let outcome = handle_skill_step_failure(
            &state,
            &task,
            &step,
            &actions,
            &[
                "skill(fragile_skill)".to_string(),
                "skill(next_skill)".to_string(),
            ],
            &mut loop_state,
            0,
            1,
            1,
            "Run two ordered operations.",
            "Run two ordered operations",
            &test_policy(),
            "fragile_skill",
            Some(&serde_json::json!({})),
            err,
            "skill",
        )
        .await
        .expect_err("non-recoverable failure should return resume context error");

        let (user_error, payload) =
            crate::parse_resume_context_error(&outcome).expect("resume context payload");
        assert!(!user_error.trim().is_empty());
        assert!(payload
            .get("resume_context")
            .and_then(|v| v.get("remaining_steps"))
            .and_then(|v| v.as_array())
            .is_some_and(|steps| steps.len() == 1));
        assert!(payload
            .get("resume_context")
            .and_then(|v| v.get("failed_step"))
            .and_then(|v| v.get("error"))
            .and_then(|v| v.as_str())
            .is_some_and(|value| value.contains("missing field")));
    }

    #[tokio::test]
    async fn missing_target_failure_without_fallback_publishes_failure_only() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "not_found",
                "error_text": "path not found: missing.md"
            })
        );
        let step = failed_step("step_1", "system_basic", &err);
        let actions = vec![crate::AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({"action":"read_range","path":"missing.md"}),
        }];

        let stop = handle_skill_step_failure(
            &state,
            &task,
            &step,
            &actions,
            &["skill(system_basic)".to_string()],
            &mut loop_state,
            0,
            1,
            1,
            "Read missing.md, then recover if needed.",
            "Read missing.md, then recover if needed.",
            &test_policy(),
            "system_basic",
            Some(&serde_json::json!({"action":"read_range","path":"missing.md"})),
            &err,
            "skill",
        )
        .await
        .expect("recoverable skill failure should not raise resume context");

        assert_eq!(stop.as_deref(), Some("recoverable_failure_finalize"));
        assert!(loop_state.has_recoverable_failure_context);
        let failed_error = loop_state
            .output_vars
            .get("failed_step.error")
            .map(String::as_str)
            .unwrap_or_default();
        assert!(
            failed_error.contains("target path was not found"),
            "failed_error={failed_error}"
        );
        assert_eq!(loop_state.progress_messages.len(), 1);
        assert!(loop_state.progress_messages[0].contains("telegram.progress.step_failed"));
        assert!(loop_state.progress_messages[0].contains("system_basic"));
        assert!(!loop_state
            .progress_messages
            .iter()
            .any(|message| message.contains("telegram.progress.retry_")));
    }

    #[tokio::test]
    async fn recoverable_protocol_failure_publishes_replan_progress() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "unsupported_action",
                "error_text": "unknown action: check_exists"
            })
        );
        let step = failed_step("step_1", "system_basic", &err);
        let actions = vec![crate::AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({"action":"check_exists","path":"README.md"}),
        }];

        let stop = handle_skill_step_failure(
            &state,
            &task,
            &step,
            &actions,
            &["skill(system_basic)".to_string()],
            &mut loop_state,
            0,
            1,
            1,
            "Check README.md exists.",
            "Check README.md exists.",
            &test_policy(),
            "system_basic",
            Some(&serde_json::json!({"action":"check_exists","path":"README.md"})),
            &err,
            "skill",
        )
        .await
        .expect("protocol failure should be recoverable");

        assert_eq!(stop.as_deref(), Some("recoverable_failure_continue_round"));
        assert_eq!(loop_state.progress_messages.len(), 2);
        assert!(loop_state.progress_messages[0].contains("telegram.progress.step_failed"));
        assert!(loop_state.progress_messages[0].contains("system_basic"));
        assert!(loop_state.progress_messages[1].contains("telegram.progress.retry_replan"));
    }

    #[tokio::test]
    async fn validation_failure_records_failed_output_and_advances_recipe_repair() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            repair_count: 0,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };

        let detail = "http response missing expected text=ops-repair-ok";
        let output = "status=200\nops-repair-bad\n";
        let outcome = handle_skill_step_success(
            &state,
            &task,
            &mut loop_state,
            "skill:http_basic:{\"action\":\"get\"}",
            &ok_step("step_1", "http_basic", output),
            1,
            1,
            "http_basic",
            "skill",
            "",
            &serde_json::json!({ "action": "get", "url": "http://127.0.0.1:62078/" }),
            output,
            crate::execution_recipe::ActionEffect::validate(),
            crate::execution_recipe::ValidationObservation::Failed(detail.to_string()),
            None,
            None,
        )
        .await
        .expect("skill step outcome");

        assert!(!outcome.ended_with_user_visible_output);
        assert!(!outcome.continue_in_round);
        assert_eq!(
            outcome.stop_signal.as_deref(),
            Some("recoverable_failure_continue_round")
        );
        assert_eq!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Repair
        );
        assert_eq!(loop_state.execution_recipe.repair_count, 1);
        assert!(loop_state.has_tool_or_skill_output);
        assert_eq!(
            loop_state
                .output_vars
                .get("failed_step.error")
                .map(String::as_str),
            Some(detail)
        );
        assert_eq!(
            loop_state
                .output_vars
                .get("skill.http_basic.error")
                .map(String::as_str),
            Some(detail)
        );
        assert_eq!(
            loop_state
                .output_vars
                .get("failed_step.action")
                .map(String::as_str),
            Some("skill(http_basic)")
        );
        assert!(loop_state
            .history_compact
            .iter()
            .any(|line| line.contains("validation_failed")
                && line.contains("http response missing expected text=ops-repair-ok")));
        assert!(loop_state.successful_action_fingerprints.is_empty());
        assert_eq!(loop_state.executed_step_results.len(), 1);
        assert!(
            loop_state.last_recipe_progress_phase
                == Some(crate::execution_recipe::ExecutionRecipePhase::Repair)
        );
        assert!(loop_state
            .subtask_results
            .iter()
            .any(|line| line.contains("subtask#1 skill(http_basic): success")));
    }

    #[tokio::test]
    async fn run_cmd_validation_failed_marker_advances_recipe_repair_without_success_fingerprint() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 2;
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            repair_count: 0,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };

        let output = "VALIDATION_FAILED\n";
        let outcome = handle_skill_step_success(
            &state,
            &task,
            &mut loop_state,
            "skill:run_cmd:{\"command\":\"curl\"}",
            &ok_step("step_2", "run_cmd", output),
            2,
            1,
            "run_cmd",
            "skill",
            "",
            &serde_json::json!({ "command": "curl -s http://127.0.0.1:62078/" }),
            output,
            crate::execution_recipe::ActionEffect::validate(),
            crate::execution_recipe::ValidationObservation::Failed("VALIDATION_FAILED".to_string()),
            None,
            None,
        )
        .await
        .expect("skill step outcome");

        assert_eq!(
            outcome.stop_signal.as_deref(),
            Some("recoverable_failure_continue_round")
        );
        assert_eq!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Repair
        );
        assert_eq!(loop_state.execution_recipe.repair_count, 1);
        assert!(loop_state.successful_action_fingerprints.is_empty());
        assert!(loop_state
            .history_compact
            .iter()
            .any(|line| line.contains("skill=run_cmd")
                && line.contains("validation_failed=VALIDATION_FAILED")));
        assert_eq!(
            loop_state
                .output_vars
                .get("failed_step.error")
                .map(String::as_str),
            Some("VALIDATION_FAILED")
        );
        assert!(loop_state
            .subtask_results
            .iter()
            .any(|line| line.contains("subtask#2 skill(run_cmd): success")));
    }

    #[tokio::test]
    async fn successful_external_workspace_step_records_scope_progress() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: false,
            max_repairs: 2,
            saw_inspect: true,
            ..Default::default()
        };

        handle_skill_step_success(
            &state,
            &task,
            &mut loop_state,
            "skill:read_file:{\"path\":\"/opt/other-project/main.rs\"}",
            &ok_step("step_3", "read_file", "fn main() {}\n"),
            3,
            1,
            "read_file",
            "skill",
            "",
            &serde_json::json!({ "path": "/opt/other-project/main.rs" }),
            "fn main() {}\n",
            crate::execution_recipe::ActionEffect::observe(),
            crate::execution_recipe::ValidationObservation::Passed,
            None,
            Some("/opt/other-project/main.rs"),
        )
        .await
        .expect("skill step outcome");

        assert!(loop_state.execution_recipe.saw_external_target);
    }

    #[tokio::test]
    async fn successful_greenfield_creation_step_records_scope_progress() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            saw_inspect: true,
            ..Default::default()
        };

        handle_skill_step_success(
            &state,
            &task,
            &mut loop_state,
            "skill:write_file:{\"path\":\"tools/demo/main.rs\"}",
            &ok_step("step_4", "write_file", "ok"),
            4,
            1,
            "write_file",
            "skill",
            "",
            &serde_json::json!({ "path": "tools/demo/main.rs", "content": "fn main() {}\n" }),
            "ok",
            crate::execution_recipe::ActionEffect::mutate(),
            crate::execution_recipe::ValidationObservation::Passed,
            None,
            None,
        )
        .await
        .expect("skill step outcome");

        assert!(loop_state.execution_recipe.saw_greenfield_creation);
    }
}

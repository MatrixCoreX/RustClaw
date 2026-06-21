use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

use super::{
    build_resume_context_error, classify_skill_failure_recovery, ensure_task_running,
    register_failed_step_output, register_file_path_output, register_step_output,
    remember_written_file_alias, AgentLoopGuardPolicy, AppState, ClaimedTask, LoopState,
    SkillActionOutcome, WriteFileEffectivePath, TASK_CANCELED_ERR,
};
use crate::{repo, run_skill_with_runner_outcome};

#[path = "skill_execution_preflight.rs"]
mod skill_execution_preflight;

use skill_execution_preflight::{
    contract_matrix_action_policy_error, contract_matrix_arg_policy_error,
    handle_preflight_argument_failure, structured_observation_path_argument_error,
    unresolved_runtime_template_argument_error, validate_skill_output_contract,
};

#[cfg(test)]
use skill_execution_preflight::{
    contains_unresolved_runtime_template_arg, preflight_failure_metadata,
};

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

fn remember_skill_metadata(loop_state: &mut LoopState, normalized_skill: &str) {
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), normalized_skill.to_string());
}

fn skill_extra_requests_user_input(extra: Option<&Value>) -> bool {
    let Some(obj) = extra.and_then(Value::as_object) else {
        return false;
    };
    obj.get("requires_user_input")
        .or_else(|| obj.get("needs_user_input"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn matrix_admitted_external_evidence_output(
    state: &AppState,
    normalized_skill: &str,
    action_args: &Value,
    out: &str,
    structured_extra: Option<&Value>,
) -> Option<String> {
    let extra = structured_extra?;
    let registry = state.get_skills_registry()?;
    let canonical = registry.resolve_canonical(normalized_skill)?;
    let entry = registry.get(canonical)?;
    let admission = entry.matrix_admission.as_ref()?;
    let requires_admission = entry.matrix_admission.is_some()
        || entry.kind == claw_core::skill_registry::SkillKind::External
        || entry
            .external_bundle_dir
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    if !requires_admission || !admission.eligible {
        return None;
    }
    let action = extra
        .get("action")
        .and_then(Value::as_str)
        .or_else(|| action_args.get("action").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if !registry.matrix_admission_eligible(canonical, action) {
        return None;
    }
    let extractor_kind = admission
        .extractor_kind
        .as_deref()
        .map(normalize_machine_token)
        .unwrap_or_else(|| "structured_json".to_string());
    if extractor_kind != "structured_json" {
        return None;
    }
    if !admission
        .required_extra_fields
        .iter()
        .all(|field| admitted_extra_field_exists(extra, field))
    {
        return None;
    }
    let mut payload = serde_json::Map::new();
    if let Some(action) = action {
        payload.insert("action".to_string(), json!(action));
    }
    payload.insert("text".to_string(), json!(out));
    payload.insert("extra".to_string(), extra.clone());
    payload.insert(
        "_matrix_admission".to_string(),
        json!({
            "schema_version": 1,
            "source": "skills_registry",
            "skill": canonical,
            "eligible": true,
            "extractor_kind": extractor_kind,
            "declared_actions": &admission.declared_actions,
            "evidence_sources": &admission.evidence_sources,
            "required_extra_fields": &admission.required_extra_fields,
            "admission_version": admission.admission_version.as_deref(),
        }),
    );
    Some(Value::Object(payload).to_string())
}

fn structured_extra_evidence_output(out: &str, structured_extra: Option<&Value>) -> Option<String> {
    let extra = structured_extra?;
    if extra.is_null() {
        return None;
    }
    Some(
        json!({
            "text": out,
            "extra": extra,
        })
        .to_string(),
    )
}

fn register_structured_extra_file_path_outputs(
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    normalized_skill: &str,
    structured_extra: Option<&Value>,
) {
    let Some(extra) = structured_extra.and_then(Value::as_object) else {
        return;
    };
    let mut paths = Vec::new();
    if let Some(path) = extra
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        paths.push(path.to_string());
    }
    if let Some(outputs) = extra.get("outputs").and_then(Value::as_array) {
        for item in outputs {
            let Some(path) = item
                .get("path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if !paths.iter().any(|existing| existing == path) {
                paths.push(path.to_string());
            }
        }
    }
    for path in paths {
        let mut source = String::from("skill");
        source.push('.');
        source.push_str(normalized_skill);
        source.push('.');
        source.push_str("extra");
        register_file_path_output(loop_state, global_step, step_in_round, &source, &path);
    }
}

fn admitted_extra_field_exists(extra: &Value, field: &str) -> bool {
    let mut field = field.trim();
    if field.is_empty() {
        return false;
    }
    field = field.strip_prefix("extra.").unwrap_or(field);
    field = field.strip_prefix("extra/").unwrap_or(field);
    field = field.trim_matches('.');
    if field.is_empty() || field == "extra" {
        return true;
    }
    let mut current = extra;
    for segment in field.split('.') {
        let segment = segment.trim();
        if segment.is_empty() {
            return false;
        }
        let Some(next) = current.get(segment) else {
            return false;
        };
        current = next;
    }
    !current.is_null()
}

fn normalize_machine_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
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

fn structured_read_request_path<'a>(normalized_skill: &str, args: &'a Value) -> Option<&'a str> {
    match normalized_skill {
        "read_file" | "list_dir" => args.get("path").and_then(Value::as_str),
        "fs_basic" | "system_basic" => {
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if matches!(
                action.as_str(),
                "read_text_range"
                    | "read_range"
                    | "list_dir"
                    | "inventory_dir"
                    | "count_entries"
                    | "count_inventory"
                    | "extract_field"
                    | "extract_fields"
                    | "structured_keys"
            ) {
                args.get("path").and_then(Value::as_str)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn path_stays_within_workspace(workspace_root: &Path, input: &str) -> bool {
    let normalized_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let raw = Path::new(input);
    if raw
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }
    let candidate = if raw.is_absolute() {
        PathBuf::from(raw)
    } else {
        normalized_root.join(raw)
    };
    let normalized_candidate = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.clone());
    normalized_candidate.starts_with(normalized_root)
}

fn auto_sudo_structured_read_targets_outside_workspace(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    structured_read_request_path(normalized_skill, args)
        .map(|path| !path_stays_within_workspace(&state.skill_rt.workspace_root, path))
        .unwrap_or(false)
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
    if auto_sudo_structured_read_targets_outside_workspace(state, normalized_skill, args) {
        return None;
    }
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
        obj.remove(super::CLAWD_USER_NAMED_OUTPUT_PATH_ARG);
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
    let journal_step_execution = matrix_admitted_external_evidence_output(
        state,
        normalized_skill,
        action_args,
        out,
        structured_extra,
    )
    .or_else(|| structured_extra_evidence_output(out, structured_extra))
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
    let user_visible_err = crate::skills::normalize_skill_error_for_user(normalized_skill, err);
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
        &user_visible_err,
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
    if let Some(err) = contract_matrix_action_policy_error(
        state,
        loop_state,
        normalized_skill,
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
    if let Some(err) = contract_matrix_arg_policy_error(loop_state, normalized_skill, &exec_args) {
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
    let step_execution =
        crate::executor::execute_step(&format!("step_{global_step}"), action, || {
            let structured_validation_slot = Arc::clone(&structured_validation_slot);
            let structured_extra_slot = Arc::clone(&structured_extra_slot);
            let exec_args_for_run = exec_args_for_run.clone();
            async move {
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
#[path = "skill_execution_tests.rs"]
mod tests;

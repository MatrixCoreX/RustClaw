use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use tracing::info;

use super::skill_execution_preflight::{
    evidence_policy_action_policy_error, handle_preflight_argument_failure,
};
use super::{
    compose_policy_block_delivery, handle_skill_step_success, log_step_journal_summary,
    record_hook_evaluation_observation, record_permission_request_hook,
    record_post_tool_use_hook_observations, register_failed_step_output, AppState, ClaimedTask,
    LoopState,
};
use crate::agent_engine::{
    append_delivery_message, append_progress_hint, build_safe_skill_args_summary,
    encode_progress_i18n, publish_agent_loop_user_input_checkpoint_progress, support,
    CLAWD_CONTINUE_ON_ERROR_ARG, CLAWD_LITERAL_COMMAND_ARG, CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG,
    CLAWD_MISSING_TARGET_REPAIRABLE_ARG, CLAWD_RUNTIME_ASYNC_JOB_START_ARG,
    CLAWD_USER_NAMED_OUTPUT_PATH_ARG, PROGRESS_ARGS_SUMMARY_MAX_LEN,
};
use crate::run_skill_with_runner_outcome;

const AUTO_SUDO_SKILL_REF: &str = "run_cmd:auto_sudo_retry";
const AUTO_SUDO_FINGERPRINT_PREFIX: &str = "skill:run_cmd:auto_sudo_retry:";
const SUDO_CMD: &str = "sudo";
const SUDO_NON_INTERACTIVE_FLAG: &str = "-n";

fn auto_sudo_skill_trace() -> String {
    ["skill", "(", AUTO_SUDO_SKILL_REF, ")"].concat()
}

fn auto_sudo_call_skill_trace() -> String {
    ["call_skill", "(", "auto_sudo_retry", ")"].concat()
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
        "{} {} {} {} {} {} {} | {} | {} {} {}",
        SUDO_CMD,
        SUDO_NON_INTERACTIVE_FLAG,
        "sh",
        "-c",
        shell_single_quote(
            r#"dir=$1; for item in "$dir"/* "$dir"/.[!.]* "$dir"/..?*; do [ -e "$item" ] || continue; basename "$item"; done"#
        ),
        "sh",
        shell_single_quote(path),
        "sort",
        "head",
        "-n",
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
        "tail" => Some(format!(
            "{} {} {} {} {} {}",
            SUDO_CMD, SUDO_NON_INTERACTIVE_FLAG, "tail", "-n", n, quoted_path
        )),
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

pub(super) fn build_auto_sudo_retry_args(
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
        obj.remove(CLAWD_CONTINUE_ON_ERROR_ARG);
        obj.remove(CLAWD_LITERAL_COMMAND_ARG);
        obj.remove(CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG);
        obj.remove(CLAWD_MISSING_TARGET_REPAIRABLE_ARG);
        obj.remove(CLAWD_RUNTIME_ASYNC_JOB_START_ARG);
        obj.remove(CLAWD_USER_NAMED_OUTPUT_PATH_ARG);
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

async fn auto_sudo_retry_failed_delivery(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    err: &str,
) -> String {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let detail = crate::truncate_for_agent_trace(err.trim());
    let contract = crate::fallback::UserResponseContract::tool_failure(
        "auto_sudo_retry_failed",
        user_text,
        "auto_sudo_retry",
        vec![
            "failure_stage: auto_sudo_retry".to_string(),
            "retry_mode: sudo_non_interactive".to_string(),
            format!("error_excerpt: {detail}"),
            "message_key: clawd.agent_loop.auto_sudo_retry_failed".to_string(),
        ],
        vec![
            "boundary:do_not_claim_retry_succeeded".to_string(),
            "boundary:observed_facts_only_one_recovery_step".to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
    )
    .await
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

pub(super) fn skill_error_observation_or_raw(skill: &str, err: &str) -> String {
    crate::skills::skill_error_machine_observation(skill, err).unwrap_or_else(|| err.to_string())
}

pub(super) fn skill_error_progress_token(_skill: &str, err: &str) -> String {
    if let Some(structured) = crate::skills::parse_structured_skill_error(err) {
        return format!(
            "error_kind={}",
            crate::truncate_for_agent_trace(structured.error_kind.trim())
        );
    }
    if let Some(policy_block) = crate::skills::parse_policy_block_error(err) {
        return format!(
            "reason_code={}",
            crate::truncate_for_agent_trace(policy_block.reason_code.trim())
        );
    }
    if crate::skills::read_file_not_found_path(err).is_some() {
        return "error_kind=not_found".to_string();
    }
    compact_progress_error(err)
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
    append_progress_hint(
        state,
        task,
        &mut loop_state.progress_messages,
        encode_progress_i18n(
            "telegram.progress.step_failed",
            &[
                ("step", step.as_str()),
                ("skill", normalized_skill),
                ("error", error.as_str()),
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
    append_progress_hint(
        state,
        task,
        &mut loop_state.progress_messages,
        encode_progress_i18n(key, &[]),
    );
}

pub(super) fn publish_failure_recovery_progress(
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

pub(super) async fn try_auto_sudo_retry_after_permission_denied(
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
    let retry_trace_kind = "auto_sudo_retry";
    if let Some(err) = evidence_policy_action_policy_error(
        state,
        loop_state,
        "run_cmd",
        &retry_args,
        retry_trace_kind,
    ) {
        let outcome = handle_preflight_argument_failure(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            "run_cmd",
            &retry_args,
            &err,
            retry_trace_kind,
        );
        return Ok(Some(outcome.stop_signal));
    }
    let pre_tool_use_evaluation = crate::agent_hooks::pre_tool_use_outcome_for_state(
        state,
        &task.task_id,
        "run_cmd",
        &retry_args,
    )
    .await;
    record_hook_evaluation_observation(
        loop_state,
        "run_cmd",
        global_step,
        step_in_round,
        &pre_tool_use_evaluation,
    );
    if pre_tool_use_evaluation.requires_confirmation() {
        let permission_evaluation = record_permission_request_hook(
            state,
            task,
            loop_state,
            "run_cmd",
            &pre_tool_use_evaluation.outcome.action_ref,
            global_step,
            step_in_round,
        )
        .await;
        if permission_evaluation.requires_background_wait() {
            support::publish_agent_loop_checkpoint_progress(
                state,
                task,
                loop_state,
                "hook_background_wait",
            );
            return Ok(Some(Some("hook_background_wait".to_string())));
        }
        if permission_evaluation.outcome.decision_kind()
            == Some(crate::policy_decision::PolicyDecision::Deny)
        {
            return Ok(Some(Some("hook_permission_denied".to_string())));
        }
        publish_agent_loop_user_input_checkpoint_progress(
            state,
            task,
            loop_state,
            "hook_confirmation_required",
            "run_cmd",
            &pre_tool_use_evaluation.outcome.action_ref,
            &retry_args,
        );
        return Ok(Some(Some("hook_confirmation_required".to_string())));
    }
    if pre_tool_use_evaluation.requires_background_wait() {
        support::publish_agent_loop_checkpoint_progress(
            state,
            task,
            loop_state,
            "hook_background_wait",
        );
        return Ok(Some(Some("hook_background_wait".to_string())));
    }
    if let Some(err) =
        crate::agent_hooks::structured_error_for_outcome(&pre_tool_use_evaluation.outcome)
    {
        let outcome = handle_preflight_argument_failure(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            "run_cmd",
            &retry_args,
            &err,
            retry_trace_kind,
        );
        return Ok(Some(outcome.stop_signal));
    }
    let progress_error = skill_error_progress_token(normalized_skill, err);
    publish_failure_recovery_progress(
        state,
        task,
        loop_state,
        step_in_round,
        normalized_skill,
        &progress_error,
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
    record_post_tool_use_hook_observations(
        state,
        task,
        loop_state,
        "run_cmd",
        &retry_args,
        global_step,
        step_in_round,
        retry_step.status,
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
            let args_summary =
                build_safe_skill_args_summary(&retry_args, PROGRESS_ARGS_SUMMARY_MAX_LEN);
            let auto_sudo_fingerprint = format!("{}{}", AUTO_SUDO_FINGERPRINT_PREFIX, retry_args);
            let outcome = handle_skill_step_success(
                state,
                task,
                loop_state,
                &auto_sudo_fingerprint,
                &retry_step,
                global_step,
                step_in_round,
                "run_cmd",
                retry_trace_kind,
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
            let retry_error_observation = skill_error_observation_or_raw("run_cmd", &retry_err);
            let auto_sudo_step_trace = auto_sudo_skill_trace();
            crate::append_subtask_result(
                &mut loop_state.subtask_results,
                global_step,
                &auto_sudo_step_trace,
                false,
                &retry_error_observation,
            );
            register_failed_step_output(
                loop_state,
                global_step,
                step_in_round,
                "skill.run_cmd",
                &auto_sudo_step_trace,
                &retry_error_observation,
            );
            loop_state.executed_step_results.push(retry_step.clone());
            let auto_sudo_call_trace = auto_sudo_call_skill_trace();
            log_step_journal_summary(
                task,
                loop_state.round_no,
                step_in_round,
                &auto_sudo_call_trace,
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
                append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, message);
                return Ok(Some(Some("policy_block_user_visible".to_string())));
            }
            let message =
                auto_sudo_retry_failed_delivery(state, task, user_text, &retry_error_observation)
                    .await;
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, message);
            loop_state.history_compact.push(format!(
                "round={} step={} skill={} permission_denied_auto_sudo_retry failed error={}",
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_agent_trace(&retry_error_observation)
            ));
            Ok(Some(Some(
                "auto_sudo_retry_failed_user_visible".to_string(),
            )))
        }
    }
}

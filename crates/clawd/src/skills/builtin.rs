use serde_json::Value;
use std::io::{Read as IoRead, Seek as IoSeek, SeekFrom, Write as IoWrite};
use std::path::{Component, Path, PathBuf};

use crate::{AppState, ClaimedTask};

#[path = "builtin_child_task_patch.rs"]
mod builtin_child_task_patch;
#[path = "builtin_run_cmd.rs"]
mod builtin_run_cmd;
#[path = "builtin_schedule.rs"]
mod builtin_schedule;
#[path = "builtin_workspace_mutation.rs"]
mod builtin_workspace_mutation;
#[path = "builtin_workspace_patch.rs"]
mod builtin_workspace_patch;
use builtin_child_task_patch::execute_child_task_patch;
#[cfg(test)]
use builtin_run_cmd::parse_run_cmd_suggestion_payload;
#[cfg(test)]
pub(crate) use builtin_run_cmd::run_safe_command;
pub(crate) use builtin_run_cmd::run_safe_command_with_sandbox;
use builtin_run_cmd::{
    command_has_shell_background_operator, looks_detached_background_command,
    run_cmd_checkpoint_claim_markers, run_cmd_claims_runtime_checkpoint_without_async_start,
    run_safe_command_detailed, start_async_command, suggest_command_for_run_cmd,
    suggested_command_from_args, RunSafeCommandError,
};
use builtin_schedule::execute_schedule_workflow_for_task;
use builtin_workspace_mutation::run_checkpointed_workspace_mutation;
use builtin_workspace_patch::execute_workspace_patch;

fn builtin_error(
    skill: &str,
    error_kind: &str,
    error_text: impl Into<String>,
    requested_path: Option<&str>,
    resolved_path: Option<&Path>,
    extra: Option<Value>,
) -> String {
    let error_text = error_text.into();
    let mut payload = extra.unwrap_or(Value::Null);
    if !payload.is_object() {
        payload = serde_json::json!({});
    }
    if let Some(object) = payload.as_object_mut() {
        if let Some(path) = requested_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            object.insert(
                "requested_path".to_string(),
                Value::String(path.to_string()),
            );
        }
        if let Some(path) = resolved_path {
            object.insert(
                "resolved_path".to_string(),
                Value::String(path.display().to_string()),
            );
        }
    }
    crate::skills::structured_skill_error_from_parts(
        skill,
        error_kind,
        &error_text,
        Some(std::env::consts::OS),
        Some(payload),
    )
}

fn io_error_kind(err: &std::io::Error) -> &'static str {
    match err.kind() {
        std::io::ErrorKind::NotFound => "not_found",
        std::io::ErrorKind::PermissionDenied => "permission_denied",
        std::io::ErrorKind::AlreadyExists => "already_exists",
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => "invalid_args",
        std::io::ErrorKind::IsADirectory => "is_directory",
        std::io::ErrorKind::NotADirectory => "not_a_directory",
        _ => "io_error",
    }
}

fn io_builtin_error(
    skill: &str,
    operation: &str,
    err: &std::io::Error,
    requested_path: Option<&str>,
    resolved_path: Option<&Path>,
) -> String {
    let target = resolved_path
        .map(|path| path.display().to_string())
        .or_else(|| requested_path.map(str::to_string))
        .unwrap_or_else(|| "<unknown>".to_string());
    builtin_error(
        skill,
        io_error_kind(err),
        format!("{operation} failed for {target}: {err}"),
        requested_path,
        resolved_path,
        None,
    )
}

pub(crate) async fn execute_builtin_skill_for_task(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: &Value,
) -> Result<String, String> {
    if skill_name != "schedule" {
        // Phase 1.2: 带着 task 走 `_with_task`，这样 run_cmd 的 NL2CMD 路径
        // 能走完整的 LLM gateway（provider fallback / audit / model_io 日志）。
        return execute_builtin_skill_with_task(state, Some(task), skill_name, args).await;
    }
    let map = ensure_args_object(args)?;
    let action = required_string(map, "action")?.trim().to_ascii_lowercase();
    if action != "compile" {
        return execute_schedule_workflow_for_task(state, task, map, args, &action).await;
    }
    ensure_only_keys(map, &["action", "text"])?;
    let text = required_string(map, "text")?;
    let intent = crate::schedule_service::parse_schedule_intent(state, task, text)
        .await
        .ok_or_else(|| "schedule_intent_not_detected".to_string())?;
    serde_json::to_string(&intent).map_err(|e| format!("serialize schedule intent failed: {e}"))
}

/// Test-only helper: 没有 `task` 上下文时调用 builtin skill。
///
/// 生产链路一律走 [`execute_builtin_skill_for_task`]，不要再新增对此函数的依赖
/// （会绕过 LLM 预算 / model_io 日志 / provider fallback 链路）。
#[cfg(test)]
pub(crate) async fn execute_builtin_skill(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> Result<String, String> {
    execute_builtin_skill_with_task(state, None, skill_name, args).await
}

pub(crate) async fn execute_builtin_skill_with_task(
    state: &AppState,
    task: Option<&ClaimedTask>,
    skill_name: &str,
    args: &Value,
) -> Result<String, String> {
    let policy_token = format!("skill:{skill_name}");
    if !state
        .skill_rt
        .tools_policy
        .is_allowed(&policy_token, state.core.active_provider_type.as_deref())
    {
        return Err(crate::skills::policy_block_error(
            "skill_policy_denied",
            vec![
                format!("skill: {skill_name}"),
                format!("policy_token: {policy_token}"),
            ],
            vec![
                "action=execute_builtin_skill".to_string(),
                "policy=tools_policy".to_string(),
                "required_decision=allow".to_string(),
            ],
        ));
    }

    let map = ensure_args_object(args)?;

    match skill_name {
        "read_file" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.skill_rt.workspace_root,
                path,
                builtin_allows_path_outside_workspace(state, task),
            )?;
            if real_path.is_dir() {
                return Err(builtin_error(
                    "read_file",
                    "is_directory",
                    format!(
                        "read_file requires a file, but target is a directory: {}",
                        real_path.display()
                    ),
                    Some(path),
                    Some(&real_path),
                    None,
                ));
            }
            let bytes = std::fs::read(&real_path).map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    format!(
                        "{}{}",
                        super::READ_FILE_NOT_FOUND_PREFIX,
                        real_path.display()
                    )
                } else {
                    io_builtin_error("read_file", "read file", &err, Some(path), Some(&real_path))
                }
            })?;
            let clip = if bytes.len() > crate::MAX_READ_FILE_BYTES {
                &bytes[..crate::MAX_READ_FILE_BYTES]
            } else {
                &bytes
            };
            Ok(String::from_utf8_lossy(clip).to_string())
        }
        "write_file" => {
            ensure_only_keys(
                map,
                &[
                    "path",
                    "content",
                    "append",
                    "mode",
                    "create_parents",
                    "parents",
                ],
            )?;
            let path = required_string(map, "path")?;
            let content = required_string(map, "content")?;
            let append = write_file_append_flag(map)?;
            let create_parents = write_file_create_parents_flag(map)?;
            if content.len() > crate::MAX_WRITE_FILE_BYTES {
                return Err(builtin_error(
                    "write_file",
                    "content_too_large",
                    format!("content too large: {} bytes", content.len()),
                    Some(path),
                    None,
                    Some(serde_json::json!({
                        "content_bytes": content.len(),
                        "max_content_bytes": crate::MAX_WRITE_FILE_BYTES,
                    })),
                ));
            }
            let effective_path =
                crate::ensure_default_file_path(&state.skill_rt.workspace_root, path);
            let real_path = resolve_workspace_path(
                &state.skill_rt.workspace_root,
                &effective_path,
                builtin_allows_path_outside_workspace(state, task),
            )?;
            let action = if append { "append_text" } else { "write_text" };
            run_checkpointed_workspace_mutation(
                &state.skill_rt.workspace_root,
                builtin_task_id(task),
                action,
                &real_path,
                || {
                    if create_parents {
                        if let Some(parent) = real_path.parent() {
                            std::fs::create_dir_all(parent).map_err(|err| {
                                io_builtin_error(
                                    "write_file",
                                    "create_parent",
                                    &err,
                                    Some(path),
                                    Some(parent),
                                )
                            })?;
                        }
                    }
                    if append {
                        let prepend_line_separator =
                            append_needs_line_separator(&real_path, content).map_err(|err| {
                                io_builtin_error(
                                    "write_file",
                                    "inspect_before_append",
                                    &err,
                                    Some(path),
                                    Some(&real_path),
                                )
                            })?;
                        let mut file = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&real_path)
                            .map_err(|err| {
                                io_builtin_error(
                                    "write_file",
                                    "open_for_append",
                                    &err,
                                    Some(path),
                                    Some(&real_path),
                                )
                            })?;
                        if prepend_line_separator {
                            file.write_all(b"\n").map_err(|err| {
                                io_builtin_error(
                                    "write_file",
                                    "append_line_separator",
                                    &err,
                                    Some(path),
                                    Some(&real_path),
                                )
                            })?;
                        }
                        file.write_all(content.as_bytes()).map_err(|err| {
                            io_builtin_error(
                                "write_file",
                                "append_file",
                                &err,
                                Some(path),
                                Some(&real_path),
                            )
                        })?;
                    } else {
                        std::fs::write(&real_path, content).map_err(|err| {
                            io_builtin_error(
                                "write_file",
                                "write_file",
                                &err,
                                Some(path),
                                Some(&real_path),
                            )
                        })?;
                    }
                    Ok(())
                },
            )
        }
        "list_dir" => {
            ensure_only_keys(map, &["path", "names_only", "limit", "max_entries"])?;
            let path = optional_string(map, "path").unwrap_or(".");
            let max_entries = optional_usize(map, "limit")
                .or_else(|| optional_usize(map, "max_entries"))
                .map(|value| value.clamp(1, 200))
                .unwrap_or(200);
            let requested_path = resolve_workspace_path(
                &state.skill_rt.workspace_root,
                path,
                builtin_allows_path_outside_workspace(state, task),
            )?;
            let real_path = if requested_path.is_dir() {
                requested_path
            } else {
                match crate::delivery_utils::resolve_directory_locator_for_execution(
                    path,
                    &state.skill_rt.default_locator_search_dir,
                    state.skill_rt.locator_scan_max_depth,
                    state.skill_rt.locator_scan_max_files,
                ) {
                    Some(crate::delivery_utils::DirectoryLocatorExecutionResolution::Resolved(
                        directory,
                    )) => directory,
                    Some(
                        crate::delivery_utils::DirectoryLocatorExecutionResolution::MultipleCandidates(
                            candidates,
                        ),
                    ) => {
                        let candidates = candidates
                            .into_iter()
                            .map(|candidate| candidate.display().to_string())
                            .collect::<Vec<_>>()
                            ;
                        return Err(builtin_error(
                            "list_dir",
                            "ambiguous_target",
                            format!(
                                "directory locator matched multiple candidates: {}",
                                candidates.join("; ")
                            ),
                            Some(path),
                            None,
                            Some(serde_json::json!({ "candidates": candidates })),
                        ));
                    }
                    Some(crate::delivery_utils::DirectoryLocatorExecutionResolution::NotFound) => {
                        return Err(builtin_error(
                            "list_dir",
                            "not_found",
                            format!("directory not found under system root and project root: {path}"),
                            Some(path),
                            Some(&requested_path),
                            None,
                        ));
                    }
                    None => requested_path,
                }
            };
            if !real_path.exists() {
                return Err(builtin_error(
                    "list_dir",
                    "not_found",
                    format!("directory not found: {}", real_path.display()),
                    Some(path),
                    Some(&real_path),
                    None,
                ));
            }
            if !real_path.is_dir() {
                return Err(builtin_error(
                    "list_dir",
                    "not_a_directory",
                    format!("list_dir requires a directory: {}", real_path.display()),
                    Some(path),
                    Some(&real_path),
                    None,
                ));
            }
            let mut items = Vec::new();
            for entry in std::fs::read_dir(&real_path).map_err(|err| {
                io_builtin_error("list_dir", "read_dir", &err, Some(path), Some(&real_path))
            })? {
                let e = entry.map_err(|err| {
                    io_builtin_error(
                        "list_dir",
                        "read directory entry",
                        &err,
                        Some(path),
                        Some(&real_path),
                    )
                })?;
                let name = e.file_name();
                let mut label = name.to_string_lossy().to_string();
                if e.path().is_dir() {
                    label.push('/');
                }
                items.push(label);
                if items.len() >= 200 {
                    break;
                }
            }
            items.sort();
            items.truncate(max_entries);
            Ok(items.join("\n"))
        }
        "run_cmd" => {
            ensure_only_keys(
                map,
                &[
                    "action",
                    "command",
                    "cwd",
                    "request_text",
                    "suggested_params",
                    "suggest_once",
                    "llm_suggest_once",
                    "timeout_seconds",
                    "idle_timeout_seconds",
                    "max_output_bytes",
                    "async_start",
                    "poll_after_seconds",
                    "expires_in_seconds",
                    "_clawd_async_job_id",
                    "_clawd_async_job_dir",
                    "_clawd_async_poll_after_seconds",
                    "_clawd_async_expires_at",
                ],
            )?;
            let cwd = optional_string(map, "cwd").unwrap_or(".");
            let cwd_path = resolve_workspace_path(
                &state.skill_rt.workspace_root,
                cwd,
                builtin_allows_path_outside_workspace(state, task),
            )?;
            let request_text = optional_string(map, "request_text")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let _suggest_once = map
                .get("suggest_once")
                .and_then(|v| v.as_bool())
                .or_else(|| map.get("llm_suggest_once").and_then(|v| v.as_bool()))
                .unwrap_or(true);
            let mut command = optional_string(map, "command")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| suggested_command_from_args(map))
                .unwrap_or_default();
            if command.trim().is_empty() {
                if let Some(ref natural_request) = request_text {
                    command = suggest_command_for_run_cmd(
                        state,
                        task,
                        natural_request,
                        &cwd_path,
                        None,
                        None,
                    )
                    .await?;
                } else {
                    return Err(
                        "command must be string (or provide request_text for NL2CMD)".to_string(),
                    );
                }
            }
            let sanitized_command = crate::bootstrap::sanitize_command_before_execute(
                &state.policy.command_intent,
                &command,
            );
            if sanitized_command.is_empty() {
                return Err("empty command after sanitize".to_string());
            }
            if sanitized_command != command.trim() {
                tracing::info!(
                    "run_cmd sanitized command: before={} after={}",
                    crate::truncate_for_log(&command),
                    crate::truncate_for_log(&sanitized_command)
                );
            }
            let timeout_seconds = map
                .get("timeout_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(state.skill_rt.cmd_timeout_seconds);
            let idle_timeout_seconds = map
                .get("idle_timeout_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(state.skill_rt.cmd_idle_timeout_seconds);
            let max_output_bytes = map
                .get("max_output_bytes")
                .and_then(|v| v.as_u64())
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(state.skill_rt.cmd_max_output_bytes);
            let async_start = map
                .get("async_start")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            let unmanaged_detached_background =
                looks_detached_background_command(&sanitized_command);
            let faked_runtime_checkpoint =
                run_cmd_claims_runtime_checkpoint_without_async_start(&sanitized_command);
            if !async_start && (unmanaged_detached_background || faked_runtime_checkpoint) {
                return Err(builtin_error(
                    "run_cmd",
                    "async_start_required",
                    "run_cmd_async_start_required",
                    None,
                    None,
                    Some(serde_json::json!({
                        "message_key": "clawd.run_cmd.async_start_required",
                        "required_args": [
                            "async_start",
                            "poll_after_seconds",
                            "expires_in_seconds"
                        ],
                        "detected_machine_fields": run_cmd_checkpoint_claim_markers(&sanitized_command),
                        "has_background_operator": command_has_shell_background_operator(&sanitized_command),
                        "unmanaged_detached_background": unmanaged_detached_background,
                        "faked_runtime_checkpoint": faked_runtime_checkpoint
                    })),
                ));
            }
            if async_start {
                let job_id = required_string(map, "_clawd_async_job_id")?;
                let job_dir = required_string(map, "_clawd_async_job_dir")?;
                return start_async_command(
                    &cwd_path,
                    &sanitized_command,
                    state.skill_rt.max_cmd_length,
                    crate::skills::task_allows_sudo(state, task),
                    &job_id,
                    Path::new(&job_dir),
                    state.skill_rt.tools_policy.sandbox_mode,
                    &state.skill_rt.workspace_root,
                )
                .await
                .map_err(|err| match err {
                    RunSafeCommandError::Policy(text) => text,
                    RunSafeCommandError::Command(failure) => {
                        let extra = failure.extra(&sanitized_command, &cwd_path);
                        super::structured_skill_error_from_parts(
                            "run_cmd",
                            failure.kind,
                            &failure.message,
                            Some(std::env::consts::OS),
                            Some(extra),
                        )
                    }
                });
            }
            run_safe_command_detailed(
                &cwd_path,
                &sanitized_command,
                state.skill_rt.max_cmd_length,
                timeout_seconds,
                idle_timeout_seconds,
                max_output_bytes,
                crate::skills::task_allows_sudo(state, task),
                state.skill_rt.tools_policy.sandbox_mode,
                &state.skill_rt.workspace_root,
            )
            .await
            .map_err(|err| match err {
                RunSafeCommandError::Policy(text) => text,
                RunSafeCommandError::Command(failure) => {
                    let extra = failure.extra(&sanitized_command, &cwd_path);
                    super::structured_skill_error_from_parts(
                        "run_cmd",
                        failure.kind,
                        &failure.message,
                        Some(std::env::consts::OS),
                        Some(extra),
                    )
                }
            })
        }
        "make_dir" => {
            ensure_only_keys(map, &["path", "parents", "recursive"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.skill_rt.workspace_root,
                path,
                builtin_allows_path_outside_workspace(state, task),
            )?;
            let create_parents = optional_bool(map, "parents")
                .or_else(|| optional_bool(map, "recursive"))
                .unwrap_or(true);
            run_checkpointed_workspace_mutation(
                &state.skill_rt.workspace_root,
                builtin_task_id(task),
                "make_dir",
                &real_path,
                || {
                    let create_result = if create_parents {
                        std::fs::create_dir_all(&real_path)
                    } else {
                        std::fs::create_dir(&real_path)
                    };
                    create_result.map_err(|err| {
                        io_builtin_error(
                            "make_dir",
                            "create_dir",
                            &err,
                            Some(path),
                            Some(&real_path),
                        )
                    })
                },
            )
        }
        "remove_file" => {
            ensure_only_keys(map, &["path", "target_kind", "recursive"])?;
            let path = required_string(map, "path")?;
            let target_kind = optional_string(map, "target_kind")
                .unwrap_or_default()
                .trim();
            let recursive = optional_bool(map, "recursive").unwrap_or(false);
            let real_path = resolve_workspace_path(
                &state.skill_rt.workspace_root,
                path,
                builtin_allows_path_outside_workspace(state, task),
            )?;
            if real_path.is_dir() && !(target_kind.eq_ignore_ascii_case("directory") && recursive) {
                return Err(builtin_error(
                    "remove_file",
                    "is_directory",
                    format!(
                        "remove_file only supports files, but target is a directory: {}",
                        real_path.display()
                    ),
                    Some(path),
                    Some(&real_path),
                    None,
                ));
            }
            run_checkpointed_workspace_mutation(
                &state.skill_rt.workspace_root,
                builtin_task_id(task),
                "remove_path",
                &real_path,
                || {
                    if real_path.is_dir() {
                        std::fs::remove_dir_all(&real_path).map_err(|err| {
                            io_builtin_error(
                                "remove_file",
                                "remove_dir_all",
                                &err,
                                Some(path),
                                Some(&real_path),
                            )
                        })
                    } else {
                        std::fs::remove_file(&real_path).map_err(|err| {
                            io_builtin_error(
                                "remove_file",
                                "remove_file",
                                &err,
                                Some(path),
                                Some(&real_path),
                            )
                        })
                    }
                },
            )
        }
        "workspace_patch" => {
            if matches!(
                map.get("action").and_then(Value::as_str),
                Some("review_child_patch" | "apply_child_patch" | "reject_child_patch")
            ) {
                execute_child_task_patch(state, task, map)
            } else {
                execute_workspace_patch(state, task, map)
            }
        }
        _ => Err(format!("unknown skill: {skill_name}")),
    }
}

fn ensure_args_object(args: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    args.as_object()
        .ok_or_else(|| "skill args must be a JSON object".to_string())
}

fn ensure_only_keys(map: &serde_json::Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    for k in map.keys() {
        if !allowed.iter().any(|x| x == k) {
            return Err(format!("unexpected arg key: {k}"));
        }
    }
    Ok(())
}

fn required_string<'a>(
    map: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    map.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{key} must be string"))
}

fn optional_string<'a>(map: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(|v| v.as_str())
}

fn optional_usize(map: &serde_json::Map<String, Value>, key: &str) -> Option<usize> {
    match map.get(key)? {
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok()),
        Value::String(value) => value.trim().parse::<usize>().ok(),
        _ => None,
    }
}

fn optional_bool(map: &serde_json::Map<String, Value>, key: &str) -> Option<bool> {
    match map.get(key)? {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn write_file_append_flag(map: &serde_json::Map<String, Value>) -> Result<bool, String> {
    let append = optional_bool(map, "append");
    let Some(mode) = optional_string(map, "mode")
        .map(str::trim)
        .filter(|mode| !mode.is_empty())
    else {
        return Ok(append.unwrap_or(false));
    };
    let mode_append = match mode.to_ascii_lowercase().as_str() {
        "overwrite" | "replace" | "write" => false,
        "append" => true,
        _ => {
            return Err(builtin_error(
                "write_file",
                "invalid_args",
                "unsupported_write_mode",
                None,
                None,
                Some(serde_json::json!({
                    "field": "mode",
                    "value": mode,
                    "supported_modes": ["overwrite", "replace", "write", "append"],
                })),
            ));
        }
    };
    if let Some(append) = append {
        if append != mode_append {
            return Err(builtin_error(
                "write_file",
                "invalid_args",
                "conflicting_write_mode",
                None,
                None,
                Some(serde_json::json!({
                    "field": "mode",
                    "append": append,
                    "mode": mode,
                })),
            ));
        }
    }
    Ok(mode_append)
}

fn write_file_create_parents_flag(map: &serde_json::Map<String, Value>) -> Result<bool, String> {
    let create_parents = optional_bool(map, "create_parents");
    let parents = optional_bool(map, "parents");
    if let (Some(create_parents), Some(parents)) = (create_parents, parents) {
        if create_parents != parents {
            return Err(builtin_error(
                "write_file",
                "invalid_args",
                "conflicting_parent_create_flags",
                None,
                None,
                Some(serde_json::json!({
                    "create_parents": create_parents,
                    "parents": parents,
                })),
            ));
        }
    }
    Ok(create_parents.or(parents).unwrap_or(true))
}

fn append_needs_line_separator(path: &Path, content: &str) -> std::io::Result<bool> {
    if content.is_empty() || content.starts_with('\n') || !content.ends_with('\n') {
        return Ok(false);
    }
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    if metadata.len() == 0 {
        return Ok(false);
    }
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    Ok(!matches!(last[0], b'\n' | b'\r'))
}

fn resolve_workspace_path(
    workspace_root: &Path,
    input: &str,
    allow_path_outside_workspace: bool,
) -> Result<PathBuf, String> {
    let normalized_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let base = if Path::new(input).is_absolute() {
        PathBuf::from(input)
    } else {
        normalized_root.join(input)
    };

    if allow_path_outside_workspace {
        return Ok(base);
    }

    if base.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(crate::skills::policy_block_error(
            "path_parent_traversal",
            vec![format!("requested_path: {input}")],
            vec![
                "path_parent_traversal_allowed=false".to_string(),
                "required_path_scope=workspace".to_string(),
                "required_path_form=concrete".to_string(),
            ],
        ));
    }

    let normalized_base = base.canonicalize().unwrap_or_else(|_| base.clone());
    if !normalized_base.starts_with(&normalized_root) {
        return Err(crate::skills::policy_block_error(
            "path_outside_workspace",
            vec![
                format!("denied_path: {}", normalized_base.display()),
                format!("workspace_root: {}", normalized_root.display()),
            ],
            vec![
                "workspace_escape_allowed=false".to_string(),
                "required_auth=admin_authorized_task".to_string(),
                "safe_next_step_required=true".to_string(),
            ],
        ));
    }

    Ok(base)
}

fn builtin_allows_path_outside_workspace(state: &AppState, task: Option<&ClaimedTask>) -> bool {
    if crate::execution_isolation::is_execution_isolation_root(&state.skill_rt.workspace_root) {
        return false;
    }
    crate::skills::task_allows_path_outside_workspace(state, task)
}

fn builtin_task_id(task: Option<&ClaimedTask>) -> &str {
    task.map(|task| task.task_id.as_str())
        .unwrap_or("test-task")
}

#[cfg(test)]
#[path = "builtin_tests.rs"]
mod tests;

use serde::Deserialize;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::{AppState, ClaimedTask};

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
    ensure_only_keys(map, &["action", "text"])?;
    let action = required_string(map, "action")?.trim().to_ascii_lowercase();
    if action != "compile" {
        return Err("schedule.action must be compile".to_string());
    }
    let text = required_string(map, "text")?;
    // Phase 0.4: 优先复用 normalizer 已经解析好的 schedule_intent，避免
    // 同一次 ask 流里对同一段文本再触发一次 LLM；只有 cache miss 或文本
    // 与 normalizer 原始输入不一致时才回退到 `parse_schedule_intent`。
    let intent =
        if let Some(cached) = state.take_task_schedule_intent_if_matches(&task.task_id, text) {
            cached
        } else {
            crate::schedule_service::parse_schedule_intent(state, task, text)
                .await
                .ok_or_else(|| "schedule intent not detected".to_string())?
        };
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
                "Do not execute the blocked skill.".to_string(),
                "Explain that the current tools policy blocks this capability.".to_string(),
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
                crate::skills::task_allows_path_outside_workspace(state, task),
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
            ensure_only_keys(map, &["path", "content"])?;
            let path = required_string(map, "path")?;
            let content = required_string(map, "content")?;
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
                crate::skills::task_allows_path_outside_workspace(state, task),
            )?;
            if let Some(parent) = real_path.parent() {
                std::fs::create_dir_all(parent).map_err(|err| {
                    io_builtin_error("write_file", "mkdir", &err, Some(path), Some(parent))
                })?;
            }
            std::fs::write(&real_path, content).map_err(|err| {
                io_builtin_error(
                    "write_file",
                    "write file",
                    &err,
                    Some(path),
                    Some(&real_path),
                )
            })?;
            Ok(format!(
                "written {} bytes to {}",
                content.len(),
                real_path.display()
            ))
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
                crate::skills::task_allows_path_outside_workspace(state, task),
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
                    "command",
                    "cwd",
                    "request_text",
                    "suggested_params",
                    "suggest_once",
                    "llm_suggest_once",
                    "timeout_seconds",
                    "idle_timeout_seconds",
                    "max_output_bytes",
                ],
            )?;
            let cwd = optional_string(map, "cwd").unwrap_or(".");
            let cwd_path = resolve_workspace_path(
                &state.skill_rt.workspace_root,
                cwd,
                crate::skills::task_allows_path_outside_workspace(state, task),
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
            run_safe_command_detailed(
                &cwd_path,
                &sanitized_command,
                state.skill_rt.max_cmd_length,
                timeout_seconds,
                idle_timeout_seconds,
                max_output_bytes,
                crate::skills::task_allows_sudo(state, task),
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
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.skill_rt.workspace_root,
                path,
                crate::skills::task_allows_path_outside_workspace(state, task),
            )?;
            std::fs::create_dir_all(&real_path).map_err(|err| {
                io_builtin_error("make_dir", "create_dir", &err, Some(path), Some(&real_path))
            })?;
            Ok(format!("created directory {}", real_path.display()))
        }
        "remove_file" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.skill_rt.workspace_root,
                path,
                crate::skills::task_allows_path_outside_workspace(state, task),
            )?;
            if real_path.is_dir() {
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
            std::fs::remove_file(&real_path).map_err(|err| {
                io_builtin_error(
                    "remove_file",
                    "remove_file",
                    &err,
                    Some(path),
                    Some(&real_path),
                )
            })?;
            Ok(format!("removed {}", real_path.display()))
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

fn looks_detached_background_command(command: &str) -> bool {
    let bytes = command.as_bytes();
    let mut saw_terminal_background = false;
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte != b'&' {
            continue;
        }
        let prev = idx.checked_sub(1).and_then(|pos| bytes.get(pos)).copied();
        let next = bytes.get(idx + 1).copied();
        if prev == Some(b'&')
            || next == Some(b'&')
            || prev == Some(b'>')
            || next == Some(b'>')
            || next.is_some_and(|value| value.is_ascii_digit())
        {
            continue;
        }
        let remainder = command[idx + 1..].trim();
        if background_followup_is_safe(remainder) {
            saw_terminal_background = true;
            continue;
        }
        return false;
    }
    saw_terminal_background
}

fn background_followup_is_safe(remainder: &str) -> bool {
    let trimmed = remainder.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    ["disown", "echo ", "printf ", ":"]
        .into_iter()
        .any(|prefix| lower == prefix.trim_end() || lower.starts_with(prefix))
}

fn suggested_command_from_args(map: &serde_json::Map<String, Value>) -> Option<String> {
    map.get("suggested_params")
        .and_then(|v| v.as_object())
        .and_then(|obj| {
            obj.get("command")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
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
                "Do not access paths containing parent traversal.".to_string(),
                "Ask for a concrete path inside the workspace.".to_string(),
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
                "Do not access paths outside the workspace for non-admin tasks.".to_string(),
                "Explain the workspace boundary and one safe next step.".to_string(),
            ],
        ));
    }

    Ok(base)
}

#[derive(Debug, Clone, Copy)]
enum CommandOutputStream {
    Stdout,
    Stderr,
}

enum CommandOutputEvent {
    Chunk {
        stream: CommandOutputStream,
        bytes: Vec<u8>,
    },
    ReadError {
        stream: CommandOutputStream,
        error: String,
    },
}

fn spawn_command_pipe_reader<R>(
    mut reader: R,
    stream: CommandOutputStream,
    tx: mpsc::UnboundedSender<CommandOutputEvent>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buf = vec![0_u8; 1024];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if tx
                        .send(CommandOutputEvent::Chunk {
                            stream,
                            bytes: buf[..n].to_vec(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(CommandOutputEvent::ReadError {
                        stream,
                        error: err.to_string(),
                    });
                    break;
                }
            }
        }
    });
}

fn append_command_output(
    stream: CommandOutputStream,
    bytes: &[u8],
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    captured_bytes: &mut usize,
    max_output_bytes: usize,
    output_truncated: &mut bool,
) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let remaining = max_output_bytes.saturating_sub(*captured_bytes);
    let take = bytes.len().min(remaining);
    if take > 0 {
        match stream {
            CommandOutputStream::Stdout => stdout.extend_from_slice(&bytes[..take]),
            CommandOutputStream::Stderr => stderr.extend_from_slice(&bytes[..take]),
        }
        *captured_bytes += take;
    }
    if take < bytes.len() || *captured_bytes >= max_output_bytes {
        *output_truncated = true;
        return true;
    }
    false
}

fn record_command_output_event(
    event: CommandOutputEvent,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    captured_bytes: &mut usize,
    max_output_bytes: usize,
    output_truncated: &mut bool,
) -> Result<bool, String> {
    match event {
        CommandOutputEvent::Chunk { stream, bytes } => Ok(append_command_output(
            stream,
            &bytes,
            stdout,
            stderr,
            captured_bytes,
            max_output_bytes,
            output_truncated,
        )),
        CommandOutputEvent::ReadError { stream, error } => {
            Err(format!("read command {stream:?} failed: {error}"))
        }
    }
}

fn combine_command_output(
    stdout: &[u8],
    stderr: &[u8],
    output_truncated: bool,
) -> (String, String, String) {
    let stdout_text = String::from_utf8_lossy(stdout).to_string();
    let stderr_text = String::from_utf8_lossy(stderr).to_string();
    let mut text = String::new();
    text.push_str(&stdout_text);
    if !stderr_text.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&stderr_text);
    }
    if output_truncated {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("...");
    }
    (text, stdout_text, stderr_text)
}

#[derive(Debug, Clone)]
struct CommandRunFailure {
    kind: &'static str,
    message: String,
    exit_code: Option<i32>,
    exit_category: Option<&'static str>,
    stdout: Option<String>,
    stderr: Option<String>,
    output_truncated: bool,
}

impl CommandRunFailure {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            exit_code: None,
            exit_category: None,
            stdout: None,
            stderr: None,
            output_truncated: false,
        }
    }

    fn with_output(
        mut self,
        exit_code: i32,
        stdout: String,
        stderr: String,
        output_truncated: bool,
    ) -> Self {
        self.exit_code = Some(exit_code);
        self.exit_category = run_cmd_exit_category(exit_code);
        self.stdout = (!stdout.trim().is_empty()).then_some(stdout);
        self.stderr = (!stderr.trim().is_empty()).then_some(stderr);
        self.output_truncated = output_truncated;
        self
    }

    fn extra(&self, command: &str, cwd: &Path) -> Value {
        serde_json::json!({
            "command": command.trim(),
            "cwd": cwd.display().to_string(),
            "exit_code": self.exit_code,
            "exit_category": self.exit_category,
            "exit_classification_source": self.exit_category.map(|_| "exit_code"),
            "stdout": self.stdout,
            "stderr": self.stderr,
            "output_truncated": self.output_truncated,
        })
    }
}

fn run_cmd_exit_category(exit_code: i32) -> Option<&'static str> {
    match exit_code {
        126 => Some("command_not_executable"),
        127 => Some("command_not_found"),
        128..=255 => Some("terminated_by_signal_or_shell_status"),
        1..=125 => Some("command_reported_failure"),
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum RunSafeCommandError {
    Policy(String),
    Command(CommandRunFailure),
}

impl RunSafeCommandError {
    fn into_text(self) -> String {
        match self {
            Self::Policy(text) => text,
            Self::Command(failure) => failure.message,
        }
    }
}

impl From<String> for RunSafeCommandError {
    fn from(message: String) -> Self {
        Self::Command(CommandRunFailure::new("output_read_failed", message))
    }
}

async fn kill_shell_pid(child_pid: Option<u32>) {
    if let Some(pid) = child_pid {
        let _ = Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .status()
            .await;
    }
}

pub(crate) async fn run_safe_command(
    cwd: &Path,
    command: &str,
    max_cmd_length: usize,
    cmd_timeout_seconds: u64,
    cmd_idle_timeout_seconds: u64,
    cmd_max_output_bytes: usize,
    allow_sudo: bool,
) -> Result<String, String> {
    run_safe_command_detailed(
        cwd,
        command,
        max_cmd_length,
        cmd_timeout_seconds,
        cmd_idle_timeout_seconds,
        cmd_max_output_bytes,
        allow_sudo,
    )
    .await
    .map_err(RunSafeCommandError::into_text)
}

async fn run_safe_command_detailed(
    cwd: &Path,
    command: &str,
    max_cmd_length: usize,
    cmd_timeout_seconds: u64,
    cmd_idle_timeout_seconds: u64,
    cmd_max_output_bytes: usize,
    allow_sudo: bool,
) -> Result<String, RunSafeCommandError> {
    if command.len() > max_cmd_length {
        return Err(RunSafeCommandError::Command(CommandRunFailure::new(
            "invalid_input",
            "command too long",
        )));
    }

    if command.trim().is_empty() {
        return Err(RunSafeCommandError::Command(CommandRunFailure::new(
            "invalid_input",
            "empty command",
        )));
    }

    if !allow_sudo && command.split_whitespace().any(|p| p == "sudo") {
        return Err(RunSafeCommandError::Policy(crate::skills::policy_block_error(
            "sudo_not_allowed",
            vec!["command_requested_sudo: true".to_string()],
            vec![
                "Do not run sudo when allow_sudo is false for this task.".to_string(),
                "Explain that elevated access requires an admin-authorized run and sudo-enabled policy.".to_string(),
            ],
        )));
    }

    let mut cmd = Command::new("bash");
    cmd.arg("-lc").arg(command);
    cmd.current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Prevent host-shell locale misconfiguration from polluting command output with
    // bash startup warnings such as "setlocale: LC_ALL...".
    cmd.env_remove("LC_ALL");
    cmd.kill_on_drop(true);

    let soft_timeout = cmd_timeout_seconds.max(1);
    let idle_timeout = cmd_idle_timeout_seconds.max(1);
    let max_output_bytes = cmd_max_output_bytes.max(128);
    let detached_background = looks_detached_background_command(command);
    let wait_timeout = if detached_background {
        soft_timeout.min(3)
    } else {
        soft_timeout
    };
    let mut child = cmd.spawn().map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "spawn_failed",
            format!("run command failed: {err}"),
        ))
    })?;
    let child_pid = child.id();

    let (tx, mut rx) = mpsc::unbounded_channel();
    if let Some(stdout) = child.stdout.take() {
        spawn_command_pipe_reader(stdout, CommandOutputStream::Stdout, tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_command_pipe_reader(stderr, CommandOutputStream::Stderr, tx.clone());
    }
    drop(tx);

    let mut wait_fut = Box::pin(child.wait());
    let total_sleep = tokio::time::sleep(Duration::from_secs(wait_timeout));
    tokio::pin!(total_sleep);
    let idle_sleep = tokio::time::sleep(Duration::from_secs(idle_timeout));
    tokio::pin!(idle_sleep);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut captured_bytes = 0usize;
    let mut output_truncated = false;
    let mut output_limit_reached = false;
    let mut detached_timeout = false;
    let mut timeout_failure: Option<CommandRunFailure> = None;
    let mut status = None;
    let mut pipes_closed = false;

    loop {
        tokio::select! {
            result = &mut wait_fut => {
                status = Some(result.map_err(|err| {
                    RunSafeCommandError::Command(CommandRunFailure::new(
                        "wait_failed",
                        format!("run command failed: {err}"),
                    ))
                })?);
                while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
                    let limit_hit = record_command_output_event(
                        event,
                        &mut stdout,
                        &mut stderr,
                        &mut captured_bytes,
                        max_output_bytes,
                        &mut output_truncated,
                    )?;
                    if limit_hit {
                        output_limit_reached = true;
                        break;
                    }
                }
                break;
            }
            maybe_event = rx.recv(), if !pipes_closed => {
                let Some(event) = maybe_event else {
                    pipes_closed = true;
                    continue;
                };
                let limit_hit = record_command_output_event(
                    event,
                    &mut stdout,
                    &mut stderr,
                    &mut captured_bytes,
                    max_output_bytes,
                    &mut output_truncated,
                )?;
                idle_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(idle_timeout));
                if limit_hit {
                    output_limit_reached = true;
                    tracing::info!(
                        "run_cmd output limit reached; killing shell (max_output_bytes={}): {}",
                        max_output_bytes,
                        crate::truncate_for_log(command)
                    );
                    kill_shell_pid(child_pid).await;
                    let _ = tokio::time::timeout(Duration::from_secs(5), &mut wait_fut).await;
                    break;
                }
            }
            _ = &mut idle_sleep => {
                tracing::info!(
                    "run_cmd idle-timeout reached; killing shell (idle={}s, configured={}s): {}",
                    idle_timeout,
                    soft_timeout,
                    crate::truncate_for_log(command)
                );
                kill_shell_pid(child_pid).await;
                let _ = tokio::time::timeout(Duration::from_secs(5), &mut wait_fut).await;
                timeout_failure = Some(CommandRunFailure::new(
                    "idle_timeout",
                    format!(
                        "Command idle timed out after {} seconds without output",
                        idle_timeout
                    ),
                ));
                break;
            }
            _ = &mut total_sleep => {
                let detached_note = if detached_background {
                    "background start grace"
                } else {
                    "soft-timeout"
                };
                tracing::info!(
                    "run_cmd {} reached; killing shell (wait={}s, configured={}s): {}",
                    detached_note,
                    wait_timeout,
                    soft_timeout,
                    crate::truncate_for_log(command)
                );
                kill_shell_pid(child_pid).await;
                let _ = tokio::time::timeout(Duration::from_secs(5), &mut wait_fut).await;
                if detached_background {
                    detached_timeout = true;
                } else {
                    timeout_failure = Some(CommandRunFailure::new(
                        "timeout",
                        format!("Command timed out after {} seconds", soft_timeout),
                    ));
                }
                break;
            }
        }
    }

    if let Some(failure) = timeout_failure {
        return Err(RunSafeCommandError::Command(failure));
    }

    let (text, stdout_text, stderr_text) =
        combine_command_output(&stdout, &stderr, output_truncated);

    if output_limit_reached {
        return if text.trim().is_empty() {
            Ok(format!("exit=truncated command={}", command.trim()))
        } else {
            Ok(text)
        };
    }

    if detached_timeout {
        return if text.trim().is_empty() {
            Ok(format!("detached=1 command={}", command.trim()))
        } else {
            Ok(text)
        };
    }

    let status = status.ok_or_else(|| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "status_unavailable",
            "run command status unavailable",
        ))
    })?;
    let exit_code = status.code().unwrap_or(-1);
    if exit_code == 0 {
        if text.trim().is_empty() {
            Ok(format!("exit=0 command={}", command.trim()))
        } else {
            Ok(text)
        }
    } else if text.trim().is_empty() {
        Err(RunSafeCommandError::Command(
            CommandRunFailure::new(
                "nonzero_exit",
                format!("Command failed with exit code {}", exit_code),
            )
            .with_output(exit_code, stdout_text, stderr_text, output_truncated),
        ))
    } else {
        let mut detail = String::new();
        if !stderr_text.trim().is_empty() {
            detail.push_str("stderr:\n");
            detail.push_str(stderr_text.trim());
        }
        if !stdout_text.trim().is_empty() {
            if !detail.is_empty() {
                detail.push_str("\n\n");
            }
            detail.push_str("stdout:\n");
            detail.push_str(stdout_text.trim());
        }
        if output_truncated {
            if !detail.is_empty() && !detail.ends_with('\n') {
                detail.push('\n');
            }
            detail.push_str("...");
        }
        Err(RunSafeCommandError::Command(
            CommandRunFailure::new(
                "nonzero_exit",
                format!("Command failed with exit code {}\n{}", exit_code, detail),
            )
            .with_output(exit_code, stdout_text, stderr_text, output_truncated),
        ))
    }
}

#[derive(Debug, Deserialize)]
struct RunCmdSuggestionPayload {
    command: String,
    confidence: Option<f64>,
    reason: Option<String>,
}

fn parse_run_cmd_suggestion_payload(
    raw: &str,
) -> Result<crate::prompt_utils::ValidatedSchemaJson<RunCmdSuggestionPayload>, String> {
    crate::prompt_utils::validate_against_schema::<RunCmdSuggestionPayload>(
        raw,
        crate::prompt_utils::PromptSchemaId::RunCmdSuggestion,
    )
    .map_err(|err| format!("run_cmd NL2CMD schema validation failed: {err}"))
}

fn build_run_cmd_nl_prompt(
    request_text: &str,
    cwd: &std::path::Path,
    previous_command: Option<&str>,
    previous_error: Option<&str>,
) -> String {
    let mut prompt = String::new();
    prompt
        .push_str("You map a natural-language request to ONE executable bash command for Linux.\n");
    prompt.push_str("Return strict JSON only: {\"command\":\"...\",\"confidence\":0.0-1.0,\"reason\":\"...\"}\n");
    prompt.push_str("Rules:\n");
    prompt.push_str("- Prefer read-only and low-risk commands.\n");
    prompt.push_str("- Do not use sudo by default.\n");
    prompt.push_str("- Avoid destructive commands (rm -rf, mkfs, reboot, shutdown, kill -9).\n");
    prompt.push_str(
        "- If one command may be missing, use shell fallback in ONE line (e.g. cmd1 || cmd2).\n",
    );
    prompt.push_str("- Output only a single-line command.\n\n");
    prompt.push_str(&format!("cwd: {}\n", cwd.display()));
    prompt.push_str(&format!("request_text: {}\n", request_text.trim()));
    if let Some(prev) = previous_command {
        prompt.push_str(&format!("previous_command: {}\n", prev.trim()));
    }
    if let Some(err) = previous_error {
        prompt.push_str(&format!(
            "previous_error: {}\n",
            crate::truncate_for_log(err)
        ));
    }
    prompt
}

async fn suggest_command_for_run_cmd(
    state: &AppState,
    task: Option<&ClaimedTask>,
    request_text: &str,
    cwd: &std::path::Path,
    previous_command: Option<&str>,
    previous_error: Option<&str>,
) -> Result<String, String> {
    let prompt = build_run_cmd_nl_prompt(request_text, cwd, previous_command, previous_error);
    // Phase 1.2: 有 task 时走完整的 LLM gateway —— provider fallback /
    // audit log / model_io 日志 / per-task trace 都会统一记录；仅在没有
    // task 上下文（legacy `run_tool` / 测试路径）时回退到 first provider。
    let text = if let Some(task_ctx) = task {
        crate::llm_gateway::run_with_fallback_with_prompt_source(
            state,
            task_ctx,
            &prompt,
            "run_cmd_nl2cmd",
        )
        .await
        .map_err(|e| format!("run_cmd NL2CMD provider failed: {e}"))?
    } else {
        let provider =
            state.core.llm_providers.first().cloned().ok_or_else(|| {
                "run_cmd NL2CMD unavailable: no llm provider configured".to_string()
            })?;
        let resp = crate::call_provider_with_retry(provider, &prompt)
            .await
            .map_err(|e| format!("run_cmd NL2CMD provider failed: {e}"))?;
        resp.text
    };
    let validated = parse_run_cmd_suggestion_payload(&text)
        .map_err(|err| format!("{err}; raw={}", crate::truncate_for_log(&text)))?;
    if !validated.raw_parse_ok {
        tracing::info!(
            "run_cmd NL2CMD schema_parse_recovery normalized={}",
            validated.schema_normalized
        );
    }
    let parsed = validated.value;
    let mut command = parsed.command.trim().to_string();
    if command.contains('\n') {
        command = command
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
    }
    if command.is_empty() {
        return Err("run_cmd NL2CMD returned empty command".to_string());
    }
    if let Some(conf) = parsed.confidence {
        tracing::info!("run_cmd NL2CMD confidence={:.2}", conf);
    }
    if let Some(reason) = parsed.reason {
        tracing::info!("run_cmd NL2CMD reason={}", crate::truncate_for_log(&reason));
    }
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::{execute_builtin_skill, parse_run_cmd_suggestion_payload};
    use crate::{
        runtime::state::AppState, AgentRuntimeConfig, SkillViewsSnapshot, ToolsPolicy,
        DEFAULT_AGENT_ID,
    };
    use claw_core::config::{AgentConfig, ToolsConfig};
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_builtin_skill_{prefix}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn test_state(workspace_root: PathBuf) -> AppState {
        let skills_list = Arc::new(
            ["list_dir"]
                .into_iter()
                .map(str::to_string)
                .collect::<HashSet<_>>(),
        );
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: None,
                    skills_list,
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                workspace_root: workspace_root.clone(),
                default_locator_search_dir: workspace_root,
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

    #[tokio::test]
    async fn list_dir_accepts_names_only_arg() {
        let root = TempDirGuard::new("list_dir_names_only");
        fs::write(root.path.join("b.txt"), "b").expect("write b");
        fs::write(root.path.join("a.txt"), "a").expect("write a");

        let state = test_state(root.path.clone());
        let output = execute_builtin_skill(
            &state,
            "list_dir",
            &json!({"path": ".", "names_only": true}),
        )
        .await
        .expect("list_dir should succeed");

        assert_eq!(output, "a.txt\nb.txt");
    }

    #[tokio::test]
    async fn list_dir_accepts_structured_limit_arg() {
        let root = TempDirGuard::new("list_dir_limit");
        fs::write(root.path.join("c.txt"), "c").expect("write c");
        fs::write(root.path.join("a.txt"), "a").expect("write a");
        fs::write(root.path.join("b.txt"), "b").expect("write b");

        let state = test_state(root.path.clone());
        let output = execute_builtin_skill(&state, "list_dir", &json!({"path": ".", "limit": 2}))
            .await
            .expect("list_dir should succeed");

        assert_eq!(output, "a.txt\nb.txt");
    }

    #[tokio::test]
    async fn list_dir_missing_locator_is_error_not_success_observation() {
        let root = TempDirGuard::new("list_dir_missing_locator");
        let state = test_state(root.path.clone());
        let err = execute_builtin_skill(
            &state,
            "list_dir",
            &json!({"path": "definitely_missing_directory"}),
        )
        .await
        .expect_err("missing directory should fail");

        let structured =
            crate::skills::parse_structured_skill_error(&err).expect("structured list_dir error");
        assert_eq!(structured.skill, "list_dir");
        assert_eq!(structured.error_kind, "not_found");
        assert!(structured
            .error_text
            .contains("directory not found under system root and project root"));
        assert_eq!(
            structured
                .extra
                .as_ref()
                .and_then(|extra| extra.get("requested_path"))
                .and_then(|value| value.as_str()),
            Some("definitely_missing_directory")
        );
        assert!(crate::skills::is_recoverable_skill_error("list_dir", &err));
    }

    #[tokio::test]
    async fn list_dir_file_target_returns_structured_not_a_directory() {
        let root = TempDirGuard::new("list_dir_file_target");
        fs::write(root.path.join("target.txt"), "x").expect("write target");
        let state = test_state(root.path.clone());

        let err = execute_builtin_skill(&state, "list_dir", &json!({"path": "target.txt"}))
            .await
            .expect_err("file target should fail");

        let structured =
            crate::skills::parse_structured_skill_error(&err).expect("structured list_dir error");
        assert_eq!(structured.skill, "list_dir");
        assert_eq!(structured.error_kind, "not_a_directory");
        assert!(crate::skills::is_recoverable_skill_error("list_dir", &err));
    }

    #[tokio::test]
    async fn remove_file_missing_path_is_structured_but_not_recoverable() {
        let root = TempDirGuard::new("remove_file_missing_path");
        let state = test_state(root.path.clone());

        let err = execute_builtin_skill(&state, "remove_file", &json!({"path": "missing.txt"}))
            .await
            .expect_err("missing remove target should fail");

        let structured = crate::skills::parse_structured_skill_error(&err)
            .expect("structured remove_file error");
        assert_eq!(structured.skill, "remove_file");
        assert_eq!(structured.error_kind, "not_found");
        assert!(!crate::skills::is_recoverable_skill_error(
            "remove_file",
            &err
        ));
    }

    #[tokio::test]
    async fn run_cmd_accepts_timeout_seconds_override() {
        let root = TempDirGuard::new("run_cmd_timeout_override");
        let state = test_state(root.path.clone());
        let output = execute_builtin_skill(
            &state,
            "run_cmd",
            &json!({
                "command": "printf ok",
                "timeout_seconds": 1,
                "idle_timeout_seconds": 1,
                "max_output_bytes": 8000
            }),
        )
        .await
        .expect("run_cmd should succeed");

        assert_eq!(output, "ok");
    }

    #[tokio::test]
    async fn run_safe_command_idle_timeout_kills_silent_command() {
        let root = TempDirGuard::new("run_cmd_idle_timeout");
        let err = super::run_safe_command(&root.path, "sleep 2", 4096, 10, 1, 8000, false)
            .await
            .expect_err("silent command should hit idle timeout");

        assert!(err.contains("idle timed out"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn run_cmd_nonzero_exit_returns_structured_error() {
        let root = TempDirGuard::new("run_cmd_structured_nonzero");
        let state = test_state(root.path.clone());
        let err = execute_builtin_skill(
            &state,
            "run_cmd",
            &json!({
                "command": "printf problem >&2; exit 7",
                "timeout_seconds": 10,
                "idle_timeout_seconds": 10,
                "max_output_bytes": 8000
            }),
        )
        .await
        .expect_err("non-zero command should fail");

        let structured =
            crate::skills::parse_structured_skill_error(&err).expect("structured run_cmd error");
        assert_eq!(structured.skill, "run_cmd");
        assert_eq!(structured.error_kind, "nonzero_exit");
        assert!(structured.error_text.contains("exit code 7"));
        assert_eq!(
            structured
                .extra
                .as_ref()
                .and_then(|extra| extra.get("exit_code"))
                .and_then(|value| value.as_i64()),
            Some(7)
        );
        assert_eq!(
            structured
                .extra
                .as_ref()
                .and_then(|extra| extra.get("exit_category"))
                .and_then(|value| value.as_str()),
            Some("command_reported_failure")
        );
        assert_eq!(
            structured
                .extra
                .as_ref()
                .and_then(|extra| extra.get("exit_classification_source"))
                .and_then(|value| value.as_str()),
            Some("exit_code")
        );
        assert_eq!(
            structured
                .extra
                .as_ref()
                .and_then(|extra| extra.get("stderr"))
                .and_then(|value| value.as_str()),
            Some("problem")
        );
    }

    #[tokio::test]
    async fn run_cmd_sudo_policy_error_stays_policy_block() {
        let root = TempDirGuard::new("run_cmd_policy_sudo");
        let state = test_state(root.path.clone());
        let err = execute_builtin_skill(
            &state,
            "run_cmd",
            &json!({
                "command": "sudo id",
                "timeout_seconds": 10,
                "idle_timeout_seconds": 10,
                "max_output_bytes": 8000
            }),
        )
        .await
        .expect_err("sudo should be policy blocked");

        assert!(
            crate::skills::parse_policy_block_error(&err).is_some(),
            "policy block should stay parseable: {err}"
        );
        assert!(
            crate::skills::parse_structured_skill_error(&err).is_none(),
            "policy block should not be wrapped as run_cmd command failure"
        );
    }

    #[tokio::test]
    async fn run_cmd_command_not_found_uses_exit_code_category() {
        let root = TempDirGuard::new("run_cmd_exit_category_127");
        let state = test_state(root.path.clone());
        let err = execute_builtin_skill(
            &state,
            "run_cmd",
            &json!({
                "command": "definitely_missing_rustclaw_command_for_exit_category",
                "timeout_seconds": 10,
                "idle_timeout_seconds": 10,
                "max_output_bytes": 8000
            }),
        )
        .await
        .expect_err("missing command should fail");

        let structured =
            crate::skills::parse_structured_skill_error(&err).expect("structured run_cmd error");
        assert_eq!(structured.error_kind, "nonzero_exit");
        assert_eq!(
            structured
                .extra
                .as_ref()
                .and_then(|extra| extra.get("exit_code"))
                .and_then(|value| value.as_i64()),
            Some(127)
        );
        assert_eq!(
            structured
                .extra
                .as_ref()
                .and_then(|extra| extra.get("exit_category"))
                .and_then(|value| value.as_str()),
            Some("command_not_found")
        );
    }

    #[tokio::test]
    async fn run_safe_command_truncates_noisy_command_output() {
        let root = TempDirGuard::new("run_cmd_output_limit");
        let output = super::run_safe_command(
            &root.path,
            "python3 - <<'PY'\nprint('A' * 2000)\nPY",
            4096,
            10,
            10,
            128,
            false,
        )
        .await
        .expect("noisy command should return truncated output");

        assert!(output.ends_with("..."), "missing ellipsis: {output:?}");
        assert!(
            output.len() <= 132,
            "output should be bounded, len={}: {output:?}",
            output.len()
        );
    }

    #[test]
    fn detached_background_detection_ignores_common_redirections() {
        assert!(super::looks_detached_background_command(
            "python3 -m http.server 64884 --bind 127.0.0.1 > /dev/null 2>&1 &"
        ));
        assert!(!super::looks_detached_background_command(
            "curl -s http://127.0.0.1:8787/ >/dev/null 2>&1"
        ));
        assert!(super::looks_detached_background_command(
            "nohup python3 -m http.server 64884 >/tmp/demo.log 2>&1 & disown"
        ));
        assert!(!super::looks_detached_background_command(
            "python3 -m http.server 64884 --bind 127.0.0.1 > /dev/null 2>&1 & sleep 1 && curl -s http://127.0.0.1:64884/"
        ));
    }

    #[tokio::test]
    async fn run_safe_command_detaches_background_http_server() {
        let root = TempDirGuard::new("run_cmd_detach_http_server");
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind temp port");
        let port = listener.local_addr().expect("port").port();
        drop(listener);

        let command = format!(
            "cd {} && python3 -m http.server {port} --bind 127.0.0.1 > /dev/null 2>&1 & echo started",
            root.path.display()
        );
        let output = super::run_safe_command(&root.path, &command, 4096, 30, 30, 8000, false)
            .await
            .expect("background run_cmd should detach");
        assert!(
            output.contains("started") || output.contains("detached=1"),
            "unexpected output: {output}"
        );

        let mut connected = false;
        for _ in 0..40 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                connected = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(connected, "http server should listen on port {port}");

        let _ = std::process::Command::new("bash")
            .arg("-lc")
            .arg(format!("kill $(lsof -ti tcp:{port}) 2>/dev/null || true"))
            .status();
    }

    #[test]
    fn run_cmd_suggestion_schema_drift() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../prompts/schemas/run_cmd_suggestion.schema.json"
        ))
        .expect("schema json");
        let properties = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("properties");
        for field in ["command", "confidence", "reason"] {
            assert!(properties.contains_key(field), "missing property {field}");
        }
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("required");
        for field in ["command", "confidence", "reason"] {
            assert!(
                required.iter().any(|v| v.as_str() == Some(field)),
                "missing required field {field}"
            );
        }
    }

    #[test]
    fn run_cmd_suggestion_schema_rejects_missing_reason() {
        let err = parse_run_cmd_suggestion_payload(r#"{"command":"pwd","confidence":0.92}"#)
            .expect_err("schema should reject missing reason");
        assert!(err.contains("missing required field `reason`"));
    }

    #[test]
    fn run_cmd_suggestion_schema_rejects_extra_property() {
        let err = parse_run_cmd_suggestion_payload(
            r#"{"command":"pwd","confidence":0.92,"reason":"show cwd","extra":true}"#,
        )
        .expect_err("schema should reject unexpected property");
        assert!(err.contains("unexpected property `extra`"));
    }
}

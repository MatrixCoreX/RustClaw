use serde::Deserialize;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

use crate::{AppState, ClaimedTask};

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
    let intent = if let Some(cached) = state.take_task_schedule_intent_if_matches(&task.task_id, text) {
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
        .tools_policy
        .is_allowed(&policy_token, state.active_provider_type.as_deref())
    {
        return Err(format!("blocked by policy: {policy_token}"));
    }

    let map = ensure_args_object(args)?;

    match skill_name {
        "read_file" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            let bytes = std::fs::read(&real_path).map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    format!(
                        "{}{}",
                        super::READ_FILE_NOT_FOUND_PREFIX,
                        real_path.display()
                    )
                } else {
                    format!("read file failed: {err}")
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
                return Err(format!("content too large: {} bytes", content.len()));
            }
            let effective_path = crate::ensure_default_file_path(&state.workspace_root, path);
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                &effective_path,
                state.allow_path_outside_workspace,
            )?;
            if let Some(parent) = real_path.parent() {
                std::fs::create_dir_all(parent).map_err(|err| format!("mkdir failed: {err}"))?;
            }
            std::fs::write(&real_path, content)
                .map_err(|err| format!("write file failed: {err}"))?;
            Ok(format!(
                "written {} bytes to {}",
                content.len(),
                real_path.display()
            ))
        }
        "list_dir" => {
            ensure_only_keys(map, &["path", "names_only"])?;
            let path = optional_string(map, "path").unwrap_or(".");
            let requested_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            let real_path = if requested_path.is_dir() {
                requested_path
            } else {
                match crate::delivery_utils::resolve_directory_locator_for_execution(
                    path,
                    &state.default_locator_search_dir,
                    state.locator_scan_max_depth,
                    state.locator_scan_max_files,
                ) {
                    Some(crate::delivery_utils::DirectoryLocatorExecutionResolution::Resolved(
                        directory,
                    )) => directory,
                    Some(
                        crate::delivery_utils::DirectoryLocatorExecutionResolution::MultipleCandidates(
                            candidates,
                        ),
                    ) => {
                        let mut lines = vec![crate::i18n_t_with_default(
                            state,
                            "clawd.msg.directory.multiple_candidates",
                            "Found multiple possible directories. Please confirm which one:",
                        )];
                        lines.extend(
                            candidates
                                .into_iter()
                                .map(|candidate| candidate.display().to_string()),
                        );
                        return Ok(lines.join("\n"));
                    }
                    Some(crate::delivery_utils::DirectoryLocatorExecutionResolution::NotFound) => {
                        return Ok(crate::i18n_t_with_default(
                            state,
                            "clawd.msg.directory.not_found_dual_root",
                            "Directory not found under system root and project root.",
                        ));
                    }
                    None => requested_path,
                }
            };
            let mut items = Vec::new();
            for entry in
                std::fs::read_dir(&real_path).map_err(|err| format!("read_dir failed: {err}"))?
            {
                let e = entry.map_err(|err| format!("dir entry failed: {err}"))?;
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
                ],
            )?;
            let cwd = optional_string(map, "cwd").unwrap_or(".");
            let cwd_path = resolve_workspace_path(
                &state.workspace_root,
                cwd,
                state.allow_path_outside_workspace,
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
            let sanitized_command =
                crate::bootstrap::sanitize_command_before_execute(&state.command_intent, &command);
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
                .unwrap_or(state.cmd_timeout_seconds);
            run_safe_command(
                &cwd_path,
                &sanitized_command,
                state.max_cmd_length,
                timeout_seconds,
                state.allow_sudo,
            )
            .await
        }
        "make_dir" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            std::fs::create_dir_all(&real_path)
                .map_err(|err| format!("create_dir failed: {err}"))?;
            Ok(format!("created directory {}", real_path.display()))
        }
        "remove_file" => {
            ensure_only_keys(map, &["path"])?;
            let path = required_string(map, "path")?;
            let real_path = resolve_workspace_path(
                &state.workspace_root,
                path,
                state.allow_path_outside_workspace,
            )?;
            if real_path.is_dir() {
                return Err(
                    "remove_file only supports files; use run_cmd for directory removal"
                        .to_string(),
                );
            }
            std::fs::remove_file(&real_path).map_err(|err| format!("remove_file failed: {err}"))?;
            Ok(format!("removed {}", real_path.display()))
        }
        "chat" => execute_builtin_chat(state, task, args).await,
        _ => Err(format!("unknown skill: {skill_name}")),
    }
}

// Phase 2.2: chat skill 内置实现。
//
// 历史上 chat 走 chat-skill 子进程：拼装 system / memory / lang_hint / user_text
// 后**直接**打 OpenAI Compat 协议，绕过整个 LLM 治理链路（无 fallback / 无 retry /
// 无 circuit breaker / 无预算 / 无 model_io.log）。
//
// 现在并入 clawd 内部，复用 [`crate::llm_gateway::run_with_fallback_chat`]：
// * provider fallback：minimax 挂自动用其它家。
// * circuit breaker：连续失败的 provider 进 cooldown，不会被反复打。
// * LLM 预算：chat 也算进单任务总调用 / 总耗时。
// * model_io.log：chat 也参与统一审计日志。
//
// 兼容性：
// * 不再返回 `extra.llm` 字段（builtin skill 接口只回 `String`）。
//   原本由 `extra.llm.prompt_source` 暴露的元数据，现在通过 `model_io.log`
//   里 `prompt_source = "chat_skill_runtime"` 暴露，观测面更全。
// * temperature / max_tokens 仍然按 style 与文本长度计算，等价 chat-skill。
// * persona / memory / lang_hint / explicit system_prompt 全部保留。
// * 外部调用方仍可独立 spawn `chat-skill` 二进制（保留为 deprecated 路径）。

const DEFAULT_CHAT_SYSTEM_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/chat_skill_system_prompt.md");
const DEFAULT_CHAT_JOKE_SYSTEM_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/chat_skill_joke_system_prompt.md");
const CHAT_SYSTEM_PROMPT_LOGICAL_PATH: &str = "prompts/chat_skill_system_prompt.md";
const CHAT_JOKE_SYSTEM_PROMPT_LOGICAL_PATH: &str = "prompts/chat_skill_joke_system_prompt.md";

async fn execute_builtin_chat(
    state: &AppState,
    task: Option<&ClaimedTask>,
    args: &Value,
) -> Result<String, String> {
    let task = task.ok_or_else(|| {
        "chat skill requires task context (cannot run without claimed task)".to_string()
    })?;

    let map = ensure_args_object(args)?;
    let user_text = map
        .get("text")
        .or_else(|| map.get("prompt"))
        .or_else(|| map.get("input"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if user_text.is_empty() {
        return Err("chat skill requires non-empty args.text".to_string());
    }
    let style = map
        .get("style")
        .or_else(|| map.get("mode"))
        .and_then(|v| v.as_str())
        .unwrap_or("chat")
        .trim()
        .to_ascii_lowercase();

    let (default_template, prompt_source_default) = match style.as_str() {
        "joke" => (
            DEFAULT_CHAT_JOKE_SYSTEM_PROMPT_TEMPLATE,
            CHAT_JOKE_SYSTEM_PROMPT_LOGICAL_PATH,
        ),
        _ => (
            DEFAULT_CHAT_SYSTEM_PROMPT_TEMPLATE,
            CHAT_SYSTEM_PROMPT_LOGICAL_PATH,
        ),
    };
    let (loaded_template, resolved_path) = crate::bootstrap::load_prompt_template_for_state(
        state,
        prompt_source_default,
        default_template,
    );

    let explicit_system_prompt = map
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let prompt_source = if explicit_system_prompt.is_some() {
        "chat_inline_system_prompt".to_string()
    } else {
        format!("chat_template:{resolved_path}")
    };
    let base_system_prompt = explicit_system_prompt.unwrap_or(loaded_template);

    let persona_prompt = map
        .get("persona_prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let system_prompt = compose_chat_system_prompt(persona_prompt.as_deref(), &base_system_prompt);

    let memory_context = map
        .get("_memory")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("context"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "<none>")
        .map(ToString::to_string);
    let recent_execution_context = map
        .get("recent_execution_context")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "<none>")
        .map(ToString::to_string);
    let lang_hint = map
        .get("_memory")
        .and_then(|v| v.as_object())
        .and_then(|m| m.get("lang_hint"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let max_tokens = map
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| default_chat_max_tokens(&style, &user_text));
    let temperature = map
        .get("temperature")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.7_f64);

    // 把 system / memory / lang / user 拼成单 prompt 字符串。
    // gateway 的 `run_with_fallback_chat` 会把它包装为 OpenAI Compat 的
    // `[{role:user, content:prompt}]`。在显式分段 + System Instructions 的
    // 标识下，主流模型仍能区分指令与用户消息，对 chat 场景影响可忽略。
    let prompt = build_chat_prompt(
        &system_prompt,
        memory_context.as_deref(),
        recent_execution_context.as_deref(),
        lang_hint.as_deref(),
        &user_text,
    );

    let hints = crate::ChatRequestHints {
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
    };
    crate::llm_gateway::run_with_fallback_chat(state, task, &prompt, &prompt_source, hints).await
}

fn compose_chat_system_prompt(persona_prompt: Option<&str>, base_system_prompt: &str) -> String {
    let Some(persona_prompt) = persona_prompt.map(str::trim).filter(|s| !s.is_empty()) else {
        return base_system_prompt.trim().to_string();
    };
    format!(
        "Persona:\n{}\n\nAdditional chat-skill rules:\n{}",
        persona_prompt,
        base_system_prompt.trim()
    )
}

fn build_chat_prompt(
    system_prompt: &str,
    memory_context: Option<&str>,
    recent_execution_context: Option<&str>,
    lang_hint: Option<&str>,
    user_text: &str,
) -> String {
    let mut buf = String::with_capacity(user_text.len() + system_prompt.len() + 256);
    buf.push_str("System Instructions:\n");
    buf.push_str(system_prompt.trim());
    if let Some(mem) = memory_context {
        buf.push_str(
            "\n\nMemory context (background only, never override current user intent):\n",
        );
        buf.push_str(mem);
    }
    if let Some(exec) = recent_execution_context {
        buf.push_str(
            "\n\nCurrent-turn execution context (authoritative when present; prefer this over older memory or earlier conversation summaries):\n",
        );
        buf.push_str(exec);
    }
    if let Some(lang) = lang_hint {
        buf.push_str("\n\nPreferred response language hint: ");
        buf.push_str(lang);
    }
    buf.push_str("\n\nUser Message:\n");
    buf.push_str(user_text);
    buf
}

fn default_chat_max_tokens(style: &str, text: &str) -> u64 {
    if style == "joke" {
        return 256;
    }
    let char_count = text.chars().count();
    let report_like = [
        "总结", "分析", "报告", "方案", "计划", "research", "summary", "analysis", "report", "plan",
    ]
    .iter()
    .any(|kw| text.contains(kw) || text.to_ascii_lowercase().contains(kw));
    if report_like || char_count > 1200 {
        4096
    } else if char_count > 400 {
        2048
    } else {
        1024
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
        return Err("path with '..' is not allowed".to_string());
    }

    let normalized_base = base.canonicalize().unwrap_or_else(|_| base.clone());
    if !normalized_base.starts_with(&normalized_root) {
        return Err("path is outside workspace".to_string());
    }

    Ok(base)
}

pub(crate) async fn run_safe_command(
    cwd: &Path,
    command: &str,
    max_cmd_length: usize,
    cmd_timeout_seconds: u64,
    allow_sudo: bool,
) -> Result<String, String> {
    if command.len() > max_cmd_length {
        return Err("command too long".to_string());
    }

    if command.trim().is_empty() {
        return Err("empty command".to_string());
    }

    if !allow_sudo && command.split_whitespace().any(|p| p == "sudo") {
        return Err("sudo is not allowed by tools config".to_string());
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
    let detached_background = looks_detached_background_command(command);
    let wait_timeout = if detached_background {
        soft_timeout.min(3)
    } else {
        soft_timeout
    };
    let child = cmd
        .spawn()
        .map_err(|err| format!("run command failed: {err}"))?;
    let child_pid = child.id();
    let mut output_fut = Box::pin(child.wait_with_output());

    let out = match tokio::time::timeout(Duration::from_secs(wait_timeout), &mut output_fut).await {
        Ok(r) => r.map_err(|err| format!("run command failed: {err}"))?,
        Err(_) => {
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
            if let Some(pid) = child_pid {
                let _ = Command::new("kill")
                    .arg("-9")
                    .arg(pid.to_string())
                    .status()
                    .await;
            }
            if detached_background {
                match tokio::time::timeout(Duration::from_secs(5), &mut output_fut).await {
                    Ok(Ok(out)) => out,
                    Ok(Err(err)) => return Err(format!("run command failed after detach: {err}")),
                    Err(_) => {
                        return Ok(format!("detached=1 command={}", command.trim()));
                    }
                }
            } else {
                let _ = output_fut.await;
                return Err(format!("Command timed out after {} seconds", soft_timeout));
            }
        }
    };

    let stdout_text = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr_text = String::from_utf8_lossy(&out.stderr).to_string();

    let mut text = String::new();
    text.push_str(&stdout_text);
    if !stderr_text.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&stderr_text);
    }

    if text.len() > 8000 {
        text.truncate(8000);
    }

    let exit_code = out.status.code().unwrap_or(-1);
    if exit_code == 0 {
        if text.trim().is_empty() {
            Ok(format!("exit=0 command={}", command.trim()))
        } else {
            Ok(text)
        }
    } else if text.trim().is_empty() {
        Err(format!("Command failed with exit code {}", exit_code))
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
        if detail.len() > 8000 {
            detail.truncate(8000);
        }
        Err(format!(
            "Command failed with exit code {}\n{}",
            exit_code, detail
        ))
    }
}

#[derive(Debug, Deserialize)]
struct RunCmdSuggestionPayload {
    command: String,
    confidence: Option<f64>,
    reason: Option<String>,
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
        let provider = state.llm_providers.first().cloned().ok_or_else(|| {
            "run_cmd NL2CMD unavailable: no llm provider configured".to_string()
        })?;
        let resp = crate::call_provider_with_retry(provider, &prompt)
            .await
            .map_err(|e| format!("run_cmd NL2CMD provider failed: {e}"))?;
        resp.text
    };
    let parsed = crate::parse_llm_json_extract_or_any::<RunCmdSuggestionPayload>(&text)
        .ok_or_else(|| {
            format!(
                "run_cmd NL2CMD invalid json: {}",
                crate::truncate_for_log(&text)
            )
        })?;
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
    use super::{
        build_chat_prompt, compose_chat_system_prompt, default_chat_max_tokens,
        execute_builtin_skill,
    };
    use crate::{
        runtime::state::AppState, AgentRuntimeConfig, CommandIntentRuntime, RateLimiter,
        ScheduleRuntime, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
    };
    use claw_core::config::{
        AgentConfig, MaintenanceConfig, MemoryConfig, RoutingConfig, ToolsConfig,
    };
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, RwLock};
    use std::time::{Instant, SystemTime, UNIX_EPOCH};
    use tokio::sync::Semaphore;

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
            started_at: Instant::now(),
            queue_limit: 1,
            db: crate::db_init::test_pool(),
            llm_providers: Vec::new(),
            agents_by_id: Arc::new(agents_by_id),
            skill_timeout_seconds: 30,
            skill_runner_path: PathBuf::new(),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list,
            }))),
            skill_semaphore: Arc::new(Semaphore::new(1)),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::new(60, 30))),
            llm_calls_per_task: Arc::new(Mutex::new(HashMap::new())),
            llm_elapsed_per_task: Arc::new(Mutex::new(HashMap::new())),
            llm_by_prompt_per_task: Arc::new(Mutex::new(HashMap::new())),
            task_schedule_intent_cache: Arc::new(Mutex::new(HashMap::new())),
            maintenance: MaintenanceConfig::default(),
            memory: MemoryConfig::default(),
            workspace_root: workspace_root.clone(),
            default_locator_search_dir: workspace_root,
            locator_scan_max_depth: 2,
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            active_provider_type: None,
            cmd_timeout_seconds: 30,
            max_cmd_length: 4096,
            allow_path_outside_workspace: false,
            allow_sudo: false,
            worker_task_timeout_seconds: 300,
            worker_task_heartbeat_seconds: 10,
            worker_running_no_progress_timeout_seconds: 300,
            worker_running_recovery_check_interval_seconds: 30,
            last_running_recovery_check_ts: Arc::new(Mutex::new(0)),
            routing: RoutingConfig::default(),
            persona_prompt: String::new(),
            command_intent: CommandIntentRuntime {
                all_result_suffixes: Vec::new(),
                default_locale: "zh-CN".to_string(),
                verify_enforce_enabled: false,
            },
            schedule: ScheduleRuntime {
                timezone: "Asia/Shanghai".to_string(),
                intent_prompt_template: String::new(),
                intent_prompt_source: String::new(),
                intent_rules_template: String::new(),
                locale: "zh-CN".to_string(),
                i18n_dict: HashMap::new(),
            },
            channels: crate::ChannelConfig::default(),
            http_client: reqwest::Client::new(),
            database_sqlite_path: PathBuf::new(),
            database_busy_timeout_ms: 5_000,
            self_extension: claw_core::config::SelfExtensionConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
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
    async fn run_cmd_accepts_timeout_seconds_override() {
        let root = TempDirGuard::new("run_cmd_timeout_override");
        let state = test_state(root.path.clone());
        let output = execute_builtin_skill(
            &state,
            "run_cmd",
            &json!({"command": "printf ok", "timeout_seconds": 1}),
        )
        .await
        .expect("run_cmd should succeed");

        assert_eq!(output, "ok");
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
        let output = super::run_safe_command(&root.path, &command, 4096, 30, false)
            .await
            .expect("background run_cmd should detach");
        assert!(
            output.contains("started") || output.contains("detached=1"),
            "unexpected output: {output}"
        );

        std::net::TcpStream::connect(("127.0.0.1", port)).expect("http server should listen");

        let _ = std::process::Command::new("bash")
            .arg("-lc")
            .arg(format!("kill $(lsof -ti tcp:{port}) 2>/dev/null || true"))
            .status();
    }

    // ---------- Phase 2.2: chat builtin 单元测试 ----------

    #[test]
    fn chat_default_max_tokens_grows_with_text_length_and_keywords() {
        // 短文本：1024
        assert_eq!(default_chat_max_tokens("chat", "hi"), 1024);
        // 中等文本（>400 字符）：2048
        let mid = "x".repeat(500);
        assert_eq!(default_chat_max_tokens("chat", &mid), 2048);
        // 长文本（>1200 字符）：4096
        let long = "x".repeat(1500);
        assert_eq!(default_chat_max_tokens("chat", &long), 4096);
        // 关键字触发 report_like：4096，无视短长度
        assert_eq!(default_chat_max_tokens("chat", "请帮我写一个总结"), 4096);
        assert_eq!(default_chat_max_tokens("chat", "give me a summary"), 4096);
        // joke 风格固定 256
        assert_eq!(default_chat_max_tokens("joke", "anything"), 256);
    }

    #[test]
    fn chat_compose_system_prompt_keeps_persona_when_present() {
        let composed = compose_chat_system_prompt(Some("Persona text"), "Base rules");
        assert!(composed.starts_with("Persona:\nPersona text"));
        assert!(composed.contains("Additional chat-skill rules:\nBase rules"));
    }

    #[test]
    fn chat_compose_system_prompt_falls_back_to_base_when_no_persona() {
        let composed = compose_chat_system_prompt(None, "Base rules");
        assert_eq!(composed, "Base rules");
        let composed_blank = compose_chat_system_prompt(Some("   "), "Base rules");
        assert_eq!(composed_blank, "Base rules");
    }

    #[test]
    fn chat_build_prompt_includes_all_optional_blocks_when_present() {
        let prompt = build_chat_prompt(
            "system",
            Some("memory body"),
            Some("recent body"),
            Some("zh-CN"),
            "user body",
        );
        assert!(prompt.contains("System Instructions:\nsystem"));
        assert!(prompt.contains("Memory context (background only"));
        assert!(prompt.contains("memory body"));
        assert!(prompt.contains("Current-turn execution context"));
        assert!(prompt.contains("recent body"));
        assert!(prompt.contains("Preferred response language hint: zh-CN"));
        assert!(prompt.ends_with("User Message:\nuser body"));
    }

    #[test]
    fn chat_build_prompt_omits_absent_optional_blocks() {
        let prompt = build_chat_prompt("system", None, None, None, "user body");
        assert!(!prompt.contains("Memory context"));
        assert!(!prompt.contains("Current-turn execution context"));
        assert!(!prompt.contains("Preferred response language hint"));
        assert!(prompt.contains("System Instructions:\nsystem"));
        assert!(prompt.ends_with("User Message:\nuser body"));
    }
}

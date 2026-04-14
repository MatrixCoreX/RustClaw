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
        return execute_builtin_skill(state, skill_name, args).await;
    }
    let map = ensure_args_object(args)?;
    ensure_only_keys(map, &["action", "text"])?;
    let action = required_string(map, "action")?.trim().to_ascii_lowercase();
    if action != "compile" {
        return Err("schedule.action must be compile".to_string());
    }
    let text = required_string(map, "text")?;
    let intent = crate::schedule_service::parse_schedule_intent(state, task, text)
        .await
        .ok_or_else(|| "schedule intent not detected".to_string())?;
    serde_json::to_string(&intent).map_err(|e| format!("serialize schedule intent failed: {e}"))
}

pub(crate) async fn execute_builtin_skill(
    state: &AppState,
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
                    command =
                        suggest_command_for_run_cmd(state, natural_request, &cwd_path, None, None)
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
            run_safe_command(
                &cwd_path,
                &sanitized_command,
                state.max_cmd_length,
                state.cmd_timeout_seconds,
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
    let child = cmd
        .spawn()
        .map_err(|err| format!("run command failed: {err}"))?;
    let child_pid = child.id();
    let mut output_fut = Box::pin(child.wait_with_output());

    let out = match tokio::time::timeout(Duration::from_secs(soft_timeout), &mut output_fut).await {
        Ok(r) => r.map_err(|err| format!("run command failed: {err}"))?,
        Err(_) => {
            tracing::info!(
                "run_cmd soft-timeout reached; killing command (soft={}s): {}",
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
            let _ = output_fut.await;
            return Err(format!("Command timed out after {} seconds", soft_timeout));
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
    request_text: &str,
    cwd: &std::path::Path,
    previous_command: Option<&str>,
    previous_error: Option<&str>,
) -> Result<String, String> {
    let provider = state
        .llm_providers
        .first()
        .cloned()
        .ok_or_else(|| "run_cmd NL2CMD unavailable: no llm provider configured".to_string())?;
    let prompt = build_run_cmd_nl_prompt(request_text, cwd, previous_command, previous_error);
    let resp = crate::call_provider_with_retry(provider, &prompt)
        .await
        .map_err(|e| format!("run_cmd NL2CMD provider failed: {e}"))?;
    let parsed = crate::parse_llm_json_extract_or_any::<RunCmdSuggestionPayload>(&resp.text)
        .ok_or_else(|| {
            format!(
                "run_cmd NL2CMD invalid json: {}",
                crate::truncate_for_log(&resp.text)
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
    use super::execute_builtin_skill;
    use crate::{
        runtime::state::AppState, AgentRuntimeConfig, CommandIntentRuntime, RateLimiter,
        ScheduleRuntime, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
    };
    use claw_core::config::{
        AgentConfig, MaintenanceConfig, MemoryConfig, RoutingConfig, ToolsConfig,
    };
    use rusqlite::Connection;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::fs;
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
            db: Arc::new(Mutex::new(Connection::open_in_memory().expect("open db"))),
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
            telegram_bot_token: String::new(),
            telegram_configured_bot_names: Arc::new(Vec::new()),
            whatsapp_cloud_enabled: false,
            whatsapp_api_base: String::new(),
            whatsapp_access_token: String::new(),
            whatsapp_phone_number_id: String::new(),
            whatsapp_web_enabled: false,
            whatsapp_web_bridge_base_url: String::new(),
            future_adapters_enabled: Arc::new(Vec::new()),
            wechat_send_config: None,
            feishu_send_config: None,
            lark_send_config: None,
            http_client: reqwest::Client::new(),
            database_sqlite_path: PathBuf::new(),
            database_busy_timeout_ms: 5_000,
            config_path_for_reload: String::new(),
            self_extension: claw_core::config::SelfExtensionConfig::default(),
            registry_path_for_reload: None,
            skill_switches_for_reload: Arc::new(HashMap::new()),
            initial_skills_list_for_reload: Vec::new(),
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
}

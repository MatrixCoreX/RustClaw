use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::*;

pub(super) fn looks_detached_background_command(command: &str) -> bool {
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

pub(super) fn command_has_shell_background_operator(command: &str) -> bool {
    let bytes = command.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for (idx, ch) in command.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if in_single || in_double || ch != '&' {
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
        return true;
    }

    false
}

pub(super) fn run_cmd_checkpoint_claim_markers(command: &str) -> Vec<&'static str> {
    let lower = command.to_ascii_lowercase();
    [
        ("checkpoint_id", "checkpoint_id"),
        ("poll_ref", "poll_ref"),
        ("next_check_after", "next_check_after"),
        ("status_background", "status=background"),
        ("status_background", "\"status\":\"background\""),
        ("pending_async_job", "pending_async_job"),
    ]
    .into_iter()
    .filter_map(|(field, token)| lower.contains(token).then_some(field))
    .collect()
}

pub(super) fn run_cmd_claims_runtime_checkpoint_without_async_start(command: &str) -> bool {
    command_has_shell_background_operator(command)
        && run_cmd_checkpoint_claim_markers(command).len() >= 2
}

pub(super) fn suggested_command_from_args(map: &serde_json::Map<String, Value>) -> Option<String> {
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
        CommandOutputEvent::ReadError { stream, error } => Err(format!(
            "run_cmd.output_read_failed stream={stream:?} error={error}"
        )),
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
pub(super) struct CommandRunFailure {
    pub(super) kind: &'static str,
    pub(super) message: String,
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

    pub(super) fn extra(&self, command: &str, cwd: &Path) -> Value {
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
pub(super) enum RunSafeCommandError {
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
        if kill_process_group(pid, "-9").await {
            return;
        }
        let _ = Command::new("kill")
            .arg("-9")
            .arg(pid.to_string())
            .status()
            .await;
    }
}

#[cfg(unix)]
fn place_child_in_own_process_group(cmd: &mut Command) {
    cmd.process_group(0);
}

#[cfg(not(unix))]
fn place_child_in_own_process_group(_cmd: &mut Command) {}

#[cfg(unix)]
async fn kill_process_group(pid: u32, signal: &str) -> bool {
    if pid == 0 {
        return false;
    }
    Command::new("kill")
        .arg(signal)
        .arg(format!("-{pid}"))
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(unix))]
async fn kill_process_group(_pid: u32, _signal: &str) -> bool {
    false
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

pub(super) async fn run_safe_command_detailed(
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
        return Err(RunSafeCommandError::Policy(
            crate::skills::policy_block_error(
                "sudo_not_allowed",
                vec!["command_requested_sudo: true".to_string()],
                vec![
                    "action=run_command".to_string(),
                    "requested_privilege=sudo".to_string(),
                    "required_policy=allow_sudo".to_string(),
                    "required_auth=admin_authorized_task".to_string(),
                ],
            ),
        ));
    }

    let mut cmd = Command::new("bash");
    crate::skills::apply_skill_runner_env_isolation(&mut cmd);
    cmd.arg("-lc").arg(command);
    cmd.current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Prevent host-shell locale misconfiguration from polluting command output with
    // bash startup warnings such as "setlocale: LC_ALL...".
    cmd.env_remove("LC_ALL");
    place_child_in_own_process_group(&mut cmd);
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
            format!("run_cmd.spawn_failed error={err}"),
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
                        format!("run_cmd.wait_failed error={err}"),
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
                    format!("run_cmd.idle_timeout seconds={idle_timeout}"),
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
                        format!("run_cmd.timeout seconds={soft_timeout}"),
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
            "run_cmd.status_unavailable",
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
                format!("run_cmd.nonzero_exit exit_code={exit_code}"),
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
                format!("run_cmd.nonzero_exit exit_code={exit_code}\n{detail}"),
            )
            .with_output(exit_code, stdout_text, stderr_text, output_truncated),
        ))
    }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(super) async fn start_async_command(
    cwd: &Path,
    command: &str,
    max_cmd_length: usize,
    allow_sudo: bool,
    job_id: &str,
    job_dir: &Path,
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
    if !allow_sudo && command.split_whitespace().any(|part| part == "sudo") {
        return Err(RunSafeCommandError::Policy(
            crate::skills::policy_block_error(
                "sudo_not_allowed",
                vec!["command_requested_sudo: true".to_string()],
                vec![
                    "policy_code:sudo_not_allowed".to_string(),
                    "required_capability:admin_sudo".to_string(),
                ],
            ),
        ));
    }
    std::fs::create_dir_all(job_dir).map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_dir_create_failed",
            format!("{}:{err}", "async_job_dir_create_failed"),
        ))
    })?;
    let stdout_path = job_dir.join("stdout");
    let stderr_path = job_dir.join("stderr");
    let exit_code_path = job_dir.join("exit_code");
    let started_path = job_dir.join("started_at");
    let finished_path = job_dir.join("finished_at");
    let run_script_path = job_dir.join("run.sh");
    let script = format!(
        "#!/usr/bin/env bash\nset +e\nprintf '%s\\n' \"$(date +%s)\" > {}\nbash -lc {} > {} 2> {}\ncode=$?\nprintf '%s\\n' \"$code\" > {}\nprintf '%s\\n' \"$(date +%s)\" > {}\n",
        shell_single_quote(&started_path.display().to_string()),
        shell_single_quote(command),
        shell_single_quote(&stdout_path.display().to_string()),
        shell_single_quote(&stderr_path.display().to_string()),
        shell_single_quote(&exit_code_path.display().to_string()),
        shell_single_quote(&finished_path.display().to_string()),
    );
    std::fs::write(&run_script_path, script).map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_script_write_failed",
            format!("{}:{err}", "async_job_script_write_failed"),
        ))
    })?;
    let mut cmd = Command::new("bash");
    crate::skills::apply_skill_runner_env_isolation(&mut cmd);
    cmd.arg(&run_script_path)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    place_child_in_own_process_group(&mut cmd);
    cmd.kill_on_drop(false);
    let child = cmd.spawn().map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_spawn_failed",
            format!("{}:{err}", "async_job_spawn_failed"),
        ))
    })?;
    if let Some(pid) = child.id() {
        let _ = std::fs::write(job_dir.join("pid"), pid.to_string());
    }
    drop(child);
    Ok(serde_json::json!({
        "status": "accepted",
        "job_id": job_id,
        "message_key": "clawd.task.async_job_started",
    })
    .to_string())
}

#[derive(Debug, Deserialize)]
pub(super) struct RunCmdSuggestionPayload {
    command: String,
    confidence: Option<f64>,
    reason: Option<String>,
}

pub(super) fn parse_run_cmd_suggestion_payload(
    raw: &str,
) -> Result<crate::prompt_utils::ValidatedSchemaJson<RunCmdSuggestionPayload>, String> {
    crate::prompt_utils::validate_against_schema::<RunCmdSuggestionPayload>(
        raw,
        crate::prompt_utils::PromptSchemaId::RunCmdSuggestion,
    )
    .map_err(|err| format!("run_cmd.nl2cmd_schema_validation_failed error={err}"))
}

fn build_run_cmd_nl_prompt(
    request_text: &str,
    cwd: &std::path::Path,
    previous_command: Option<&str>,
    previous_error: Option<&str>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str("instruction=Map request_text to one executable bash command for Linux.\n");
    prompt.push_str("format=Return strict JSON only: {\"command\":\"...\",\"confidence\":0.0-1.0,\"reason\":\"...\"}\n");
    prompt.push_str("rules:\n");
    prompt.push_str("rule=Prefer read-only and low-risk commands.\n");
    prompt.push_str("rule_id=no_default_sudo detail=avoid_sudo_unless_explicit_request\n");
    prompt.push_str("rule=Avoid destructive commands (rm -rf, mkfs, reboot, shutdown, kill -9).\n");
    prompt.push_str(
        "rule=If one command may be missing, use shell fallback in one line (example: cmd1 || cmd2).\n",
    );
    prompt.push_str("rule=Output only a single-line command.\n\n");
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

pub(super) async fn suggest_command_for_run_cmd(
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
        .map_err(|e| format!("run_cmd.nl2cmd_provider_failed error={e}"))?
    } else {
        let provider = state
            .core
            .llm_providers
            .first()
            .cloned()
            .ok_or_else(|| "run_cmd.nl2cmd_no_provider".to_string())?;
        let resp = crate::call_provider_with_retry(provider, &prompt)
            .await
            .map_err(|e| format!("run_cmd.nl2cmd_provider_failed error={e}"))?;
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
        return Err("run_cmd.nl2cmd_empty_command".to_string());
    }
    if let Some(conf) = parsed.confidence {
        tracing::info!("run_cmd NL2CMD confidence={:.2}", conf);
    }
    if let Some(reason) = parsed.reason {
        tracing::info!("run_cmd NL2CMD reason={}", crate::truncate_for_log(&reason));
    }
    Ok(command)
}

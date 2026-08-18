use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc;

#[path = "builtin_run_cmd_artifact.rs"]
mod output_artifact;

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
    tx: mpsc::Sender<CommandOutputEvent>,
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
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx
                        .send(CommandOutputEvent::ReadError {
                            stream,
                            error: err.to_string(),
                        })
                        .await;
                    break;
                }
            }
        }
    });
}

fn record_command_output_event(
    event: CommandOutputEvent,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    artifact_writer: &mut output_artifact::CommandOutputArtifactWriter,
) -> Result<bool, String> {
    match event {
        CommandOutputEvent::Chunk { stream, bytes } => artifact_writer
            .append(
                match stream {
                    CommandOutputStream::Stdout => output_artifact::OutputStream::Stdout,
                    CommandOutputStream::Stderr => output_artifact::OutputStream::Stderr,
                },
                &bytes,
                stdout,
                stderr,
            )
            .map_err(|error| format!("run_cmd.output_artifact_write_failed error={error}")),
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
    output_artifacts: Option<output_artifact::CommandOutputArtifactSummary>,
}

#[derive(Debug, Clone)]
pub(super) struct CommandRunSuccess {
    command_output: String,
    stdout: String,
    stderr: String,
    exit_code: i32,
    output_artifacts: Option<output_artifact::CommandOutputArtifactSummary>,
}

impl CommandRunSuccess {
    pub(super) fn machine_projection(&self, command: &str, cwd: &Path) -> Value {
        let artifact_projection = self
            .output_artifacts
            .as_ref()
            .map(|artifacts| artifacts.machine_projection(&self.command_output, self.exit_code));
        let artifact_refs = artifact_projection
            .as_ref()
            .and_then(|value| value.get("artifact_refs"))
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let range_handles = artifact_projection
            .as_ref()
            .and_then(|value| value.get("range_handles"))
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let complete = self.output_artifacts.is_none();
        serde_json::json!({
            "schema_version": 1,
            "source": "run_cmd",
            "status": "ok",
            "action": "exec",
            "command": command.trim(),
            "cwd": cwd.display().to_string(),
            "shell_mode": "bash_pipefail",
            "exit_code": self.exit_code,
            "exit_category": "success",
            "stdout": self.stdout,
            "stderr": self.stderr,
            "command_output": self.command_output,
            "output_truncated": !complete,
            "complete": complete,
            "artifacts": artifact_refs,
            "continuation": (!complete).then_some(serde_json::json!({
                "kind": "artifact_range",
                "ranges": range_handles,
            })),
        })
    }

    #[cfg(test)]
    fn legacy_test_output(self) -> String {
        self.output_artifacts
            .as_ref()
            .map(|artifacts| {
                artifacts
                    .machine_projection(&self.command_output, self.exit_code)
                    .to_string()
            })
            .unwrap_or(self.command_output)
    }
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
            output_artifacts: None,
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

    fn with_output_artifacts(
        mut self,
        output_artifacts: Option<output_artifact::CommandOutputArtifactSummary>,
    ) -> Self {
        self.output_artifacts = output_artifacts;
        self
    }

    pub(super) fn extra(&self, command: &str, cwd: &Path) -> Value {
        let mut extra = serde_json::json!({
            "command": command.trim(),
            "cwd": cwd.display().to_string(),
            "exit_code": self.exit_code,
            "exit_category": self.exit_category,
            "exit_classification_source": self.exit_category.map(|_| "exit_code"),
            "stdout": self.stdout,
            "stderr": self.stderr,
            "output_truncated": self.output_truncated,
        });
        if let (Some(object), Some(output_artifacts)) =
            (extra.as_object_mut(), self.output_artifacts.as_ref())
        {
            object.insert(
                "output_artifact_refs".to_string(),
                Value::Array(output_artifacts.artifact_refs.clone()),
            );
            object.insert(
                "output_total_bytes".to_string(),
                Value::Number((output_artifacts.total_bytes as u64).into()),
            );
        }
        extra
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
    #[cfg(test)]
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

#[cfg(test)]
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
        claw_core::config::ToolSandboxMode::DangerFull,
        claw_core::config::ToolSandboxBackend::Auto,
        cwd,
        "test-task",
    )
    .await
    .map(CommandRunSuccess::legacy_test_output)
    .map_err(RunSafeCommandError::into_text)
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) async fn run_safe_command_with_sandbox(
    cwd: &Path,
    command: &str,
    max_cmd_length: usize,
    cmd_timeout_seconds: u64,
    cmd_idle_timeout_seconds: u64,
    cmd_max_output_bytes: usize,
    allow_sudo: bool,
    sandbox_mode: claw_core::config::ToolSandboxMode,
    sandbox_backend: claw_core::config::ToolSandboxBackend,
    workspace_root: &Path,
) -> Result<String, String> {
    run_safe_command_detailed(
        cwd,
        command,
        max_cmd_length,
        cmd_timeout_seconds,
        cmd_idle_timeout_seconds,
        cmd_max_output_bytes,
        allow_sudo,
        sandbox_mode,
        sandbox_backend,
        workspace_root,
        "direct",
    )
    .await
    .map(CommandRunSuccess::legacy_test_output)
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
    sandbox_mode: claw_core::config::ToolSandboxMode,
    sandbox_backend: claw_core::config::ToolSandboxBackend,
    workspace_root: &Path,
    output_artifact_task_id: &str,
) -> Result<CommandRunSuccess, RunSafeCommandError> {
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

    if !allow_sudo && crate::skills::command_requests_sudo(command) {
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

    let mut cmd = prepare_run_cmd_process(cwd, sandbox_mode, sandbox_backend, workspace_root)?;
    crate::skills::apply_skill_runner_env_isolation(&mut cmd);
    configure_bash_command(&mut cmd, command);
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
    let mut child = cmd.spawn().map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "spawn_failed",
            format!("run_cmd.spawn_failed error={err}"),
        ))
    })?;
    let child_pid = child.id();

    let (tx, mut rx) = mpsc::channel(64);
    if let Some(stdout) = child.stdout.take() {
        spawn_command_pipe_reader(stdout, CommandOutputStream::Stdout, tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_command_pipe_reader(stderr, CommandOutputStream::Stderr, tx.clone());
    }
    drop(tx);

    let mut wait_fut = Box::pin(child.wait());
    let total_sleep = tokio::time::sleep(Duration::from_secs(soft_timeout));
    tokio::pin!(total_sleep);
    let idle_sleep = tokio::time::sleep(Duration::from_secs(idle_timeout));
    tokio::pin!(idle_sleep);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut artifact_writer = output_artifact::CommandOutputArtifactWriter::new(
        workspace_root,
        output_artifact_task_id,
        max_output_bytes,
    );
    let mut output_hard_limit_reached = false;
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
                        &mut artifact_writer,
                    )?;
                    if limit_hit {
                        output_hard_limit_reached = true;
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
                    &mut artifact_writer,
                )?;
                idle_sleep.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(idle_timeout));
                if limit_hit {
                    output_hard_limit_reached = true;
                    tracing::info!(
                        "run_cmd output artifact hard limit reached; killing shell (excerpt_bytes={}): {}",
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
                tracing::info!(
                    "run_cmd soft-timeout reached; killing shell (configured={}s): {}",
                    soft_timeout,
                    crate::truncate_for_log(command)
                );
                kill_shell_pid(child_pid).await;
                let _ = tokio::time::timeout(Duration::from_secs(5), &mut wait_fut).await;
                timeout_failure = Some(CommandRunFailure::new(
                    "timeout",
                    format!("run_cmd.timeout seconds={soft_timeout}"),
                ));
                break;
            }
        }
    }

    let output_artifacts = artifact_writer.finish().map_err(|error| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "output_artifact_finalize_failed",
            format!("run_cmd.output_artifact_finalize_failed error={error}"),
        ))
    })?;

    if let Some(failure) = timeout_failure {
        return Err(RunSafeCommandError::Command(
            failure.with_output_artifacts(output_artifacts),
        ));
    }
    if output_hard_limit_reached {
        return Err(RunSafeCommandError::Command(
            CommandRunFailure::new("output_hard_limit", "run_cmd.output_hard_limit")
                .with_output_artifacts(output_artifacts),
        ));
    }

    let output_truncated = output_artifacts.is_some();
    let (text, stdout_text, stderr_text) =
        combine_command_output(&stdout, &stderr, output_truncated);

    let status = status.ok_or_else(|| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "status_unavailable",
            "run_cmd.status_unavailable",
        ))
    })?;
    let exit_code = status.code().unwrap_or(-1);
    if exit_code == 0 {
        Ok(CommandRunSuccess {
            command_output: text,
            stdout: stdout_text,
            stderr: stderr_text,
            exit_code,
            output_artifacts,
        })
    } else if text.trim().is_empty() {
        Err(RunSafeCommandError::Command(
            CommandRunFailure::new(
                "nonzero_exit",
                format!("run_cmd.nonzero_exit exit_code={exit_code}"),
            )
            .with_output(exit_code, stdout_text, stderr_text, output_truncated)
            .with_output_artifacts(output_artifacts),
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
            .with_output(exit_code, stdout_text, stderr_text, output_truncated)
            .with_output_artifacts(output_artifacts),
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
    runtime_timeout_seconds: Option<u64>,
    retention_seconds: u64,
    terminate_grace_seconds: u64,
    allow_sudo: bool,
    job_id: &str,
    job_dir: &Path,
    sandbox_mode: claw_core::config::ToolSandboxMode,
    sandbox_backend: claw_core::config::ToolSandboxBackend,
    workspace_root: &Path,
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
    if !allow_sudo && crate::skills::command_requests_sudo(command) {
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
    let exit_code_temp_path = job_dir.join("exit_code.tmp");
    let started_path = job_dir.join("started_at");
    let started_temp_path = job_dir.join("started_at.tmp");
    let finished_path = job_dir.join("finished_at");
    let finished_temp_path = job_dir.join("finished_at.tmp");
    let run_script_path = job_dir.join("run.sh");
    let terminate_grace_seconds = terminate_grace_seconds.max(1);
    let command_runner = if let Some(runtime_timeout_seconds) = runtime_timeout_seconds {
        let runtime_timeout_seconds = runtime_timeout_seconds.max(1);
        format!(
            r#"if command -v python3 >/dev/null 2>&1; then
  python3 - {} {} {} {} {} <<'PY'
import os
import signal
import subprocess
import sys
import time

def process_tree(root_pid):
    try:
        rows = subprocess.check_output(
            ["ps", "-eo", "pid=,ppid="],
            stderr=subprocess.DEVNULL,
            text=True,
        ).splitlines()
    except (OSError, subprocess.SubprocessError):
        return [root_pid]
    children = {{}}
    for row in rows:
        parts = row.split()
        if len(parts) != 2:
            continue
        try:
            pid, parent = (int(parts[0]), int(parts[1]))
        except ValueError:
            continue
        children.setdefault(parent, []).append(pid)
    discovered = []
    pending = [root_pid]
    while pending:
        pid = pending.pop()
        if pid in discovered:
            continue
        discovered.append(pid)
        pending.extend(children.get(pid, []))
    return discovered

def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True

def signal_tree(pids, sig):
    for pid in reversed(pids):
        try:
            os.kill(pid, sig)
        except (ProcessLookupError, PermissionError):
            pass

def signal_group(root_pid, sig):
    try:
        os.killpg(root_pid, sig)
    except (ProcessLookupError, PermissionError):
        pass

limit = int(sys.argv[1])
grace = int(sys.argv[2])
command = sys.argv[3]
with open(sys.argv[4], "wb") as stdout, open(sys.argv[5], "wb") as stderr:
    process = subprocess.Popen(
        ["bash", "-o", "pipefail", "-c", command],
        stdout=stdout,
        stderr=stderr,
        start_new_session=True,
    )
    try:
        code = process.wait(timeout=limit)
    except subprocess.TimeoutExpired:
        pids = process_tree(process.pid)
        signal_group(process.pid, signal.SIGTERM)
        signal_tree(pids, signal.SIGTERM)
        deadline = time.monotonic() + grace
        while time.monotonic() < deadline and any(alive(pid) for pid in pids):
            time.sleep(0.05)
        signal_group(process.pid, signal.SIGKILL)
        signal_tree([pid for pid in pids if alive(pid)], signal.SIGKILL)
        try:
            process.wait(timeout=max(1, grace))
            code = 124
        except subprocess.TimeoutExpired:
            code = 125
sys.exit(code if code >= 0 else 128 - code)
PY
elif command -v timeout >/dev/null 2>&1; then
  timeout --foreground -k {} {} bash -o pipefail -c {} > {} 2> {}
elif command -v gtimeout >/dev/null 2>&1; then
  gtimeout --foreground -k {} {} bash -o pipefail -c {} > {} 2> {}
else
  printf '%s\n' 'portable_timeout_backend_unavailable' > {}
  exit 125
fi"#,
            runtime_timeout_seconds,
            terminate_grace_seconds,
            shell_single_quote(command),
            shell_single_quote(&stdout_path.display().to_string()),
            shell_single_quote(&stderr_path.display().to_string()),
            terminate_grace_seconds,
            runtime_timeout_seconds,
            shell_single_quote(command),
            shell_single_quote(&stdout_path.display().to_string()),
            shell_single_quote(&stderr_path.display().to_string()),
            terminate_grace_seconds,
            runtime_timeout_seconds,
            shell_single_quote(command),
            shell_single_quote(&stdout_path.display().to_string()),
            shell_single_quote(&stderr_path.display().to_string()),
            shell_single_quote(&stderr_path.display().to_string()),
        )
    } else {
        format!(
            "bash -o pipefail -c {} > {} 2> {}",
            shell_single_quote(command),
            shell_single_quote(&stdout_path.display().to_string()),
            shell_single_quote(&stderr_path.display().to_string()),
        )
    };
    let script = format!(
        r#"#!/usr/bin/env bash
set +e
printf '%s\n' "$(date +%s)" > {}
mv -f {} {}
{}
code=$?
printf '%s\n' "$(date +%s)" > {}
mv -f {} {}
printf '%s\n' "$code" > {}
mv -f {} {}
"#,
        shell_single_quote(&started_temp_path.display().to_string()),
        shell_single_quote(&started_temp_path.display().to_string()),
        shell_single_quote(&started_path.display().to_string()),
        command_runner,
        shell_single_quote(&finished_temp_path.display().to_string()),
        shell_single_quote(&finished_temp_path.display().to_string()),
        shell_single_quote(&finished_path.display().to_string()),
        shell_single_quote(&exit_code_temp_path.display().to_string()),
        shell_single_quote(&exit_code_temp_path.display().to_string()),
        shell_single_quote(&exit_code_path.display().to_string()),
    );
    crate::local_process_job::write_atomic(&run_script_path, &script).map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_script_write_failed",
            format!("{}:{err}", "async_job_script_write_failed"),
        ))
    })?;
    crate::local_process_job::write_atomic(&job_dir.join("job_id"), job_id).map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_metadata_write_failed",
            format!("async_job_metadata_write_failed:{err}"),
        ))
    })?;
    crate::local_process_job::write_atomic(
        &job_dir.join("runtime_timeout_seconds"),
        &runtime_timeout_seconds
            .map(|seconds| seconds.max(1).to_string())
            .unwrap_or_else(|| "disabled".to_string()),
    )
    .map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_metadata_write_failed",
            format!("async_job_metadata_write_failed:{err}"),
        ))
    })?;
    crate::local_process_job::write_atomic(
        &job_dir.join("terminate_grace_seconds"),
        &terminate_grace_seconds.to_string(),
    )
    .map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_metadata_write_failed",
            format!("async_job_metadata_write_failed:{err}"),
        ))
    })?;
    crate::local_process_job::write_atomic(
        &job_dir.join("retention_seconds"),
        &retention_seconds.max(1).to_string(),
    )
    .map_err(|err| {
        RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_metadata_write_failed",
            format!("async_job_metadata_write_failed:{err}"),
        ))
    })?;
    let mut cmd =
        prepare_durable_run_cmd_process(cwd, sandbox_mode, sandbox_backend, workspace_root)?;
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
    let Some(pid) = child.id() else {
        return Err(RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_pid_missing",
            "async_job_pid_missing",
        )));
    };
    if let Err(err) = crate::local_process_job::write_atomic(&job_dir.join("pid"), &pid.to_string())
    {
        let _ = crate::local_process_job::terminate_verified_process_group(job_dir, pid, "KILL");
        return Err(RunSafeCommandError::Command(CommandRunFailure::new(
            "async_job_metadata_write_failed",
            format!("async_job_pid_write_failed:{err}"),
        )));
    }
    drop(child);
    Ok(serde_json::json!({
        "schema_version": 1,
        "source": "run_cmd",
        "action": "async_start",
        "status": "accepted",
        "job_id": job_id,
        "complete": false,
        "retryable": true,
        "message_key": "clawd.task.async_job_started",
    })
    .to_string())
}

fn prepare_run_cmd_process(
    cwd: &Path,
    sandbox_mode: claw_core::config::ToolSandboxMode,
    sandbox_backend: claw_core::config::ToolSandboxBackend,
    workspace_root: &Path,
) -> Result<Command, RunSafeCommandError> {
    prepare_run_cmd_process_for_lifetime(cwd, sandbox_mode, sandbox_backend, workspace_root, false)
}

fn prepare_durable_run_cmd_process(
    cwd: &Path,
    sandbox_mode: claw_core::config::ToolSandboxMode,
    sandbox_backend: claw_core::config::ToolSandboxBackend,
    workspace_root: &Path,
) -> Result<Command, RunSafeCommandError> {
    prepare_run_cmd_process_for_lifetime(cwd, sandbox_mode, sandbox_backend, workspace_root, true)
}

pub(super) fn prepare_durable_pty_command(
    cwd: &Path,
    command: &str,
    sandbox_mode: claw_core::config::ToolSandboxMode,
    sandbox_backend: claw_core::config::ToolSandboxBackend,
    workspace_root: &Path,
) -> Result<Command, RunSafeCommandError> {
    let mut cmd =
        prepare_durable_run_cmd_process(cwd, sandbox_mode, sandbox_backend, workspace_root)?;
    crate::skills::apply_skill_runner_env_isolation(&mut cmd);
    configure_bash_command(&mut cmd, command);
    cmd.current_dir(cwd);
    Ok(cmd)
}

fn configure_bash_command(cmd: &mut Command, command: &str) {
    // Runtime commands must not source user login profiles. Besides making
    // execution host-dependent, profiles may launch interactive programs and
    // prevent a short tool call from ever reaching the requested command.
    cmd.args(["-o", "pipefail", "-c"]).arg(command);
}

fn prepare_run_cmd_process_for_lifetime(
    cwd: &Path,
    sandbox_mode: claw_core::config::ToolSandboxMode,
    sandbox_backend: claw_core::config::ToolSandboxBackend,
    workspace_root: &Path,
    durable_async: bool,
) -> Result<Command, RunSafeCommandError> {
    let request = crate::process_sandbox::ProcessSandboxRequest {
        mode: sandbox_mode,
        backend: sandbox_backend,
        workspace_root,
        execution_root: cwd,
        network: crate::process_sandbox::ProcessNetworkPolicy::Deny,
        additional_writable_paths: &[],
    };
    let prepared = if durable_async {
        crate::process_sandbox::prepare_durable_process_command("bash", request)
    } else {
        crate::process_sandbox::prepare_process_command("bash", request)
    }
    .map_err(|reason_code| {
        RunSafeCommandError::Policy(crate::skills::policy_block_error(
            reason_code,
            vec![
                format!("sandbox_mode={}", sandbox_mode.as_token()),
                format!("sandbox_backend={}", sandbox_backend.as_token()),
            ],
            vec![
                "action=run_command".to_string(),
                format!("sandbox_backend_required={}", sandbox_backend.as_token()),
            ],
        ))
    })?;
    tracing::debug!(
        sandbox_backend = prepared.backend,
        sandbox_backend_requested = sandbox_backend.as_token(),
        sandbox_mode = sandbox_mode.as_token(),
        "run_cmd_process_sandbox_prepared"
    );
    Ok(prepared.command)
}

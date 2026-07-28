//! Skill-runner: 接收 clawd 通过 stdin 投递的技能请求，分发到对应 skill
//! 二进制并把执行结果按行回写 stdout。
//!
//! P4.3 重写要点（vs 旧实现）：
//! - 全链路 tokio：`tokio::io::stdin/stdout` 替代阻塞的 `io::stdin().lock().lines()`，
//!   `tokio::process::Command` 替代 `std::process::Command`。
//! - 子进程超时改用 `tokio::time::timeout` + `Command::kill_on_drop(true)`，
//!   不再 `try_wait` + `thread::sleep(30ms)` busy-poll，CPU 占用归零。
//! - 用 `wait_with_output()` 一次性收 stdout/stderr，避免旧实现先 `try_wait`
//!   再 `wait_with_output` 时\"子进程写满 pipe buffer 阻塞\"的潜在死锁。
//! - 单进程串行处理多次请求语义保持不变（每条 stdin 行 = 一次请求）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustclaw_skill_sdk::{
    prepare_sandboxed_command, validate_response_line, LauncherKind, ProtocolResponse,
    ProtocolStatus, SandboxNetwork, SandboxProfile, SkillLaunchSpec, SkillRuntimeResolver,
    PARENT_SANDBOX_BACKEND_ENV, SKILL_STORAGE_WRITABLE_DIRECTORY_ENV,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Deserialize)]
struct SkillRequest {
    request_id: String,
    user_id: i64,
    chat_id: i64,
    user_key: Option<String>,
    skill_name: String,
    args: Value,
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct SkillResponse {
    request_id: String,
    status: String,
    text: String,
    buttons: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation: Option<Value>,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChildSkillResponse {
    request_id: Option<String>,
    status: Option<String>,
    text: Option<String>,
    buttons: Option<Value>,
    error_kind: Option<String>,
    platform: Option<String>,
    exit_code: Option<i32>,
    validation: Option<Value>,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug)]
struct ExecutionFailure {
    error_code: &'static str,
    detail: String,
    exit_code: Option<i32>,
    signal: Option<i32>,
    timed_out: bool,
    truncated: bool,
    retryable: bool,
}

impl ExecutionFailure {
    fn new(error_code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            error_code,
            detail: detail.into(),
            exit_code: None,
            signal: None,
            timed_out: false,
            truncated: false,
            retryable: false,
        }
    }

    fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

#[derive(Debug)]
struct ResponseParseFailure {
    error_code: String,
    detail: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Result<SkillRequest, _> = serde_json::from_str(trimmed);
        let resp = match parsed {
            Ok(req) => execute_skill(req).await,
            Err(err) => SkillResponse {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                error_kind: Some("invalid_input".to_string()),
                platform: Some(std::env::consts::OS.to_string()),
                exit_code: None,
                validation: None,
                extra: Some(serde_json::json!({
                    "error_code": "invalid_input",
                    "message_key": "skill_runner.invalid_input",
                    "retryable": false,
                })),
                error_text: Some(format!("invalid request: {err}")),
            },
        };

        let mut out = serde_json::to_string(&resp)?;
        out.push('\n');
        stdout.write_all(out.as_bytes()).await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn execute_skill(req: SkillRequest) -> SkillResponse {
    let timeout_secs: u64 = std::env::var("SKILL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(30);

    let child_launch = match resolve_child_launch(&req.skill_name) {
        Ok(launch) => launch,
        Err(err) => {
            return SkillResponse {
                request_id: req.request_id,
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                error_kind: Some("runner_resolution_failed".to_string()),
                platform: Some(std::env::consts::OS.to_string()),
                exit_code: None,
                validation: None,
                extra: Some(serde_json::json!({
                    "error_code": "runner_resolution_failed",
                    "message_key": "skill_runner.runner_resolution_failed",
                    "retryable": false,
                })),
                error_text: Some(err),
            }
        }
    };

    let child_req = serde_json::json!({
        "request_id": req.request_id,
        "args": req.args,
        "context": req.context,
        "user_id": req.user_id,
        "chat_id": req.chat_id,
        "user_key": req.user_key,
    });

    let timeout = Duration::from_secs(timeout_secs.min(child_launch.timeout_seconds));
    let execution = if child_launch.launcher == LauncherKind::HttpJson {
        run_http_json_skill(&child_launch, &child_req, timeout).await
    } else {
        run_child_skill(&child_launch, &child_req.to_string(), timeout).await
    };
    match execution {
        Ok(out) => {
            let parsed = parse_child_response(&out, &req.request_id, child_launch.strict_protocol);
            match parsed {
                Ok(v) => SkillResponse {
                    request_id: v.request_id.unwrap_or_else(|| "unknown".to_string()),
                    status: v.status.unwrap_or_else(|| "ok".to_string()),
                    text: v.text.unwrap_or_default(),
                    buttons: v.buttons,
                    error_kind: v.error_kind,
                    platform: v.platform,
                    exit_code: v.exit_code,
                    validation: v.validation,
                    extra: v.extra,
                    error_text: v.error_text,
                },
                Err(err) => SkillResponse {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    buttons: None,
                    error_kind: Some(err.error_code.clone()),
                    platform: Some(std::env::consts::OS.to_string()),
                    exit_code: None,
                    validation: None,
                    extra: Some(serde_json::json!({
                        "error_code": err.error_code,
                        "message_key": "skill_runner.protocol_response_invalid",
                        "retryable": false,
                    })),
                    error_text: Some(format!(
                        "invalid child response: {err}; output_bytes={}",
                        out.len()
                    )),
                },
            }
        }
        Err(err) => SkillResponse {
            request_id: req.request_id,
            status: "error".to_string(),
            text: String::new(),
            buttons: None,
            error_kind: Some(err.error_code.to_string()),
            platform: Some(std::env::consts::OS.to_string()),
            exit_code: err.exit_code,
            validation: None,
            extra: Some(serde_json::json!({
                "error_code": err.error_code,
                "message_key": format!("skill_runner.{}", err.error_code),
                "retryable": err.retryable,
                "exit_code": err.exit_code,
                "signal": err.signal,
                "timed_out": err.timed_out,
                "truncated": err.truncated,
            })),
            error_text: Some(err.detail),
        },
    }
}

#[derive(Debug, Clone)]
struct ChildLaunch {
    program: PathBuf,
    args: Vec<String>,
    working_directory: Option<PathBuf>,
    environment: BTreeMap<String, String>,
    environment_allowlist: Vec<String>,
    strict_protocol: bool,
    launcher: LauncherKind,
    remote_endpoint: Option<String>,
    runtime_network: bool,
    timeout_seconds: u64,
    installed: bool,
    sandbox_profile: SandboxProfile,
}

impl ChildLaunch {
    #[cfg(test)]
    fn legacy(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            working_directory: None,
            environment: BTreeMap::new(),
            environment_allowlist: Vec::new(),
            strict_protocol: false,
            launcher: LauncherKind::Process,
            remote_endpoint: None,
            runtime_network: false,
            timeout_seconds: 86_400,
            installed: false,
            sandbox_profile: SandboxProfile::Required,
        }
    }

    fn installed(spec: SkillLaunchSpec) -> Self {
        Self {
            program: spec.program,
            args: spec.args,
            working_directory: Some(spec.working_directory),
            environment: spec.environment,
            environment_allowlist: spec.environment_allowlist,
            strict_protocol: true,
            launcher: spec.launcher,
            remote_endpoint: spec.remote_endpoint,
            runtime_network: spec.runtime_network,
            timeout_seconds: spec.timeout_seconds,
            installed: true,
            sandbox_profile: spec.sandbox_profile,
        }
    }
}

fn resolve_child_launch(skill_name: &str) -> Result<ChildLaunch, String> {
    let package_root = std::env::var_os("RUSTCLAW_SKILL_PACKAGES_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/skill-packages"));
    SkillRuntimeResolver::new(&package_root)
        .resolve(skill_name)
        .map(ChildLaunch::installed)
        .map_err(|error| {
            format!(
                "installed receipt resolution failed: code={} detail={}",
                error.code, error.detail
            )
        })
}

fn parse_child_response(
    output: &[u8],
    request_id: &str,
    strict_protocol: bool,
) -> Result<ChildSkillResponse, ResponseParseFailure> {
    if strict_protocol {
        let response =
            validate_response_line(output, request_id).map_err(|error| ResponseParseFailure {
                error_code: error.code,
                detail: error.detail,
            })?;
        return Ok(protocol_response_to_child(response));
    }
    let text = std::str::from_utf8(output).map_err(|error| ResponseParseFailure {
        error_code: "legacy_response_utf8_invalid".to_string(),
        detail: error.to_string(),
    })?;
    let trimmed = text.trim_end_matches(['\n', '\r']);
    if trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(ResponseParseFailure {
            error_code: "legacy_multiple_stdout_records".to_string(),
            detail: "legacy child emitted multiple stdout records".to_string(),
        });
    }
    serde_json::from_str(trimmed).map_err(|error| ResponseParseFailure {
        error_code: "legacy_response_invalid".to_string(),
        detail: error.to_string(),
    })
}

impl std::fmt::Display for ResponseParseFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

fn protocol_response_to_child(response: ProtocolResponse) -> ChildSkillResponse {
    ChildSkillResponse {
        request_id: Some(response.request_id),
        status: Some(
            match response.status {
                ProtocolStatus::Ok => "ok",
                ProtocolStatus::Error => "error",
            }
            .to_string(),
        ),
        text: Some(response.text),
        buttons: response.buttons,
        error_kind: response.error_kind.or_else(|| {
            response
                .extra
                .as_ref()
                .and_then(|extra| extra.get("error_code"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        }),
        platform: response.platform,
        exit_code: response.exit_code,
        validation: response.validation,
        extra: response.extra,
        error_text: response.error_text,
    }
}

async fn run_child_skill(
    launch: &ChildLaunch,
    input_line: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ExecutionFailure> {
    let mut command = child_process_command(launch)
        .map_err(|detail| ExecutionFailure::new("child_launch_invalid", detail))?;
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| ExecutionFailure::new("child_spawn_failed", err.to_string()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{input_line}\n").as_bytes())
            .await
            .map_err(|err| ExecutionFailure::new("child_stdin_write_failed", err.to_string()))?;
        stdin
            .flush()
            .await
            .map_err(|err| ExecutionFailure::new("child_stdin_flush_failed", err.to_string()))?;
    }

    let child_pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ExecutionFailure::new("child_stdout_unavailable", "stdout pipe missing"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ExecutionFailure::new("child_stderr_unavailable", "stderr pipe missing"))?;
    let stdout_reader = tokio::spawn(read_bounded_output(stdout));
    let stderr_reader = tokio::spawn(read_bounded_output(stderr));
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => return Err(ExecutionFailure::new("child_wait_failed", err.to_string())),
        Err(_) => {
            terminate_child_process_group(child_pid).await;
            let _ = child.wait().await;
            stdout_reader.abort();
            stderr_reader.abort();
            let mut failure = ExecutionFailure::new("child_timeout", "child skill timeout");
            failure.timed_out = true;
            failure.retryable = true;
            return Err(failure);
        }
    };
    let stdout = stdout_reader
        .await
        .map_err(|error| ExecutionFailure::new("child_stdout_read_failed", error.to_string()))??;
    let stderr = stderr_reader
        .await
        .map_err(|error| ExecutionFailure::new("child_stderr_read_failed", error.to_string()))??;

    if !status.success() {
        let mut failure = ExecutionFailure::new(
            "child_nonzero_exit",
            format!("child process failed; diagnostic_bytes={}", stderr.len()),
        );
        failure.exit_code = status.code();
        failure.signal = exit_signal(&status);
        return Err(failure);
    }

    if stdout.is_empty() {
        return Err(ExecutionFailure::new(
            "child_stdout_empty",
            "child stdout is empty",
        ));
    }
    Ok(stdout)
}

fn child_process_command(launch: &ChildLaunch) -> Result<Command, String> {
    if !launch.installed {
        let mut command = Command::new(&launch.program);
        command.args(&launch.args).envs(&launch.environment);
        if let Some(working_directory) = &launch.working_directory {
            command.current_dir(working_directory);
        }
        return Ok(command);
    }
    let working_directory = launch
        .working_directory
        .as_deref()
        .ok_or_else(|| "installed launch working directory is missing".to_string())?;
    let std_command =
        if inherited_parent_sandbox_backend().is_some() || unrestricted_admin_authority() {
            let mut command = std::process::Command::new(&launch.program);
            command.current_dir(working_directory);
            command
        } else {
            let network = if launch.runtime_network {
                SandboxNetwork::Allow
            } else {
                SandboxNetwork::Deny
            };
            let writable_paths = installed_writable_paths(launch)?;
            prepare_sandboxed_command(&launch.program, working_directory, &writable_paths, network)
                .map_err(|error| {
                    format!(
                        "sandbox failed closed: code={} detail={}",
                        error.code, error.detail
                    )
                })?
                .command
        };
    let mut command = Command::new(std_command.get_program());
    command.args(std_command.get_args());
    if let Some(directory) = std_command.get_current_dir() {
        command.current_dir(directory);
    }
    command.env_clear();
    command.envs(&launch.environment);
    for key in &launch.environment_allowlist {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    for key in [
        "RUSTCLAW_UNRESTRICTED_ADMIN",
        "RUSTCLAW_ALLOW_PATH_OUTSIDE_WORKSPACE",
        "RUSTCLAW_ALLOW_SUDO",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.args(&launch.args);
    Ok(command)
}

fn inherited_parent_sandbox_backend() -> Option<&'static str> {
    let value = std::env::var(PARENT_SANDBOX_BACKEND_ENV).ok()?;
    inherited_parent_sandbox_backend_token(&value)
}

fn inherited_parent_sandbox_backend_token(value: &str) -> Option<&'static str> {
    match value {
        "bubblewrap" => Some("bubblewrap"),
        "macos_seatbelt" => Some("macos_seatbelt"),
        _ => None,
    }
}

fn installed_writable_paths(launch: &ChildLaunch) -> Result<Vec<PathBuf>, String> {
    let workspace = std::env::var_os("WORKSPACE_ROOT").map(PathBuf::from);
    let declared_storage =
        std::env::var_os(SKILL_STORAGE_WRITABLE_DIRECTORY_ENV).map(PathBuf::from);
    installed_writable_paths_from(launch, workspace.as_deref(), declared_storage.as_deref())
}

fn installed_writable_paths_from(
    launch: &ChildLaunch,
    workspace: Option<&Path>,
    declared_storage: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    let mut writable_paths = Vec::new();
    if launch.sandbox_profile == SandboxProfile::WorkspaceWrite {
        let workspace = workspace
            .ok_or_else(|| "workspace-write receipt requires WORKSPACE_ROOT".to_string())?;
        let workspace = std::fs::canonicalize(workspace)
            .map_err(|error| format!("workspace root unavailable: {error}"))?;
        if !workspace.is_dir() {
            return Err("workspace root is not a directory".to_string());
        }
        writable_paths.push(workspace);
    }
    if let Some(declared_storage) = declared_storage {
        let declared_storage = std::fs::canonicalize(declared_storage)
            .map_err(|error| format!("declared skill storage unavailable: {error}"))?;
        if !declared_storage.is_dir() {
            return Err("declared skill storage is not a directory".to_string());
        }
        if !writable_paths.contains(&declared_storage) {
            writable_paths.push(declared_storage);
        }
    }
    Ok(writable_paths)
}

async fn read_bounded_output(
    reader: impl tokio::io::AsyncRead + Unpin,
) -> Result<Vec<u8>, ExecutionFailure> {
    let mut output = Vec::new();
    reader
        .take((rustclaw_skill_sdk::MAX_PROTOCOL_LINE_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .await
        .map_err(|error| ExecutionFailure::new("child_output_read_failed", error.to_string()))?;
    if output.len() > rustclaw_skill_sdk::MAX_PROTOCOL_LINE_BYTES {
        let mut failure = ExecutionFailure::new(
            "child_output_truncated",
            format!(
                "child output exceeds {} bytes",
                rustclaw_skill_sdk::MAX_PROTOCOL_LINE_BYTES
            ),
        );
        failure.truncated = true;
        return Err(failure);
    }
    Ok(output)
}

async fn run_http_json_skill(
    launch: &ChildLaunch,
    request: &Value,
    timeout: Duration,
) -> Result<Vec<u8>, ExecutionFailure> {
    if !unrestricted_admin_authority() && !launch.runtime_network {
        return Err(ExecutionFailure::new(
            "http_runtime_network_denied",
            "http_json runtime network is not allowed by the receipt",
        ));
    }
    let endpoint = launch.remote_endpoint.as_deref().ok_or_else(|| {
        ExecutionFailure::new(
            "http_endpoint_missing",
            "http_json endpoint is missing from the verified launch spec",
        )
    })?;
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| ExecutionFailure::new("http_client_failed", error.to_string()))?;
    let mut request_builder = client.post(endpoint).json(request);
    if let Some(idempotency_key) = http_idempotency_header(request)? {
        request_builder = request_builder.header("Idempotency-Key", idempotency_key);
    }
    let mut response = request_builder.send().await.map_err(|error| {
        ExecutionFailure::new(
            if error.is_timeout() {
                "http_request_timeout"
            } else {
                "http_request_failed"
            },
            error.to_string(),
        )
        .retryable(true)
    })?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let mut failure = ExecutionFailure::new(
            "http_status_error",
            format!("http_json request returned status {status}"),
        );
        failure.exit_code = Some(i32::from(status));
        failure.retryable = status >= 500 || status == 429;
        return Err(failure);
    }
    if response
        .content_length()
        .is_some_and(|size| size > rustclaw_skill_sdk::MAX_PROTOCOL_LINE_BYTES as u64)
    {
        let mut failure =
            ExecutionFailure::new("http_response_truncated", "http_json response is oversized");
        failure.truncated = true;
        return Err(failure);
    }
    let mut output = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        ExecutionFailure::new("http_response_read_failed", error.to_string()).retryable(true)
    })? {
        if output.len().saturating_add(chunk.len()) > rustclaw_skill_sdk::MAX_PROTOCOL_LINE_BYTES {
            let mut failure =
                ExecutionFailure::new("http_response_truncated", "http_json response is oversized");
            failure.truncated = true;
            return Err(failure);
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

fn unrestricted_admin_authority() -> bool {
    environment_flag_value_is_enabled(std::env::var("RUSTCLAW_UNRESTRICTED_ADMIN").ok().as_deref())
}

fn environment_flag_value_is_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

fn http_idempotency_header(
    request: &Value,
) -> Result<Option<reqwest::header::HeaderValue>, ExecutionFailure> {
    let Some(idempotency_key) = request
        .pointer("/context/execution/idempotency_key")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    reqwest::header::HeaderValue::from_str(idempotency_key)
        .map(Some)
        .map_err(|error| ExecutionFailure::new("http_idempotency_key_invalid", error.to_string()))
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

#[cfg(unix)]
async fn terminate_child_process_group(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(format!("-{pid}"))
        .status()
        .await;
}

#[cfg(not(unix))]
async fn terminate_child_process_group(_pid: Option<u32>) {}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

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
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use skill_sdk::{
    prepare_sandboxed_command, validate_progress_frame_line, validate_response_line, LauncherKind,
    ProtocolResponse, ProtocolStatus, SandboxNetwork, SandboxProfile, SkillLaunchSpec,
    SkillRuntimeResolver, MAX_PROGRESS_FRAMES_PER_INVOCATION, MAX_PROGRESS_FRAMES_PER_SECOND,
    MAX_PROTOCOL_LINE_BYTES, PARENT_SANDBOX_BACKEND_ENV, SKILL_STORAGE_WRITABLE_DIRECTORY_ENV,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

#[derive(Debug, Deserialize)]
struct SkillRequest {
    request_id: String,
    user_id: i64,
    chat_id: i64,
    user_key: Option<String>,
    skill_name: String,
    expected_skill_version: String,
    expected_manifest_digest: String,
    expected_receipt_digest: String,
    expected_registry_generation: u64,
    expected_registry_generation_digest: Option<String>,
    expected_base_registry_digest: Option<String>,
    expected_overlay_generation_digest: Option<String>,
    expected_policy_digest: Option<String>,
    expected_admission_receipt_digest: Option<String>,
    args: Value,
    context: Option<Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ExecutionBinding {
    skill_name: String,
    version: String,
    manifest_digest: String,
    receipt_digest: String,
    registry_generation: u64,
    registry_generation_digest: Option<String>,
    base_registry_digest: Option<String>,
    overlay_generation_digest: Option<String>,
    policy_digest: Option<String>,
    admission_receipt_digest: Option<String>,
}

#[derive(Debug, Serialize)]
struct SkillResponse {
    request_id: String,
    status: String,
    text: String,
    buttons: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
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
    #[serde(alias = "error_kind")]
    error_code: Option<String>,
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
    progress_diagnostics: Option<ProgressFrameDiagnostics>,
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
            progress_diagnostics: None,
        }
    }

    fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
struct ProgressFrameDiagnostics {
    schema_version: u32,
    declared: bool,
    observed: u64,
    accepted: u64,
    dropped_invalid: u64,
    dropped_request_id_mismatch: u64,
    dropped_oversized: u64,
    dropped_rate_limited: u64,
    dropped_total_limited: u64,
    dropped_sequence_invalid: u64,
}

impl ProgressFrameDiagnostics {
    fn declared() -> Self {
        Self {
            schema_version: 1,
            declared: true,
            ..Self::default()
        }
    }
}

#[derive(Debug)]
struct ChildExecution {
    final_output: Vec<u8>,
    progress_diagnostics: ProgressFrameDiagnostics,
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
            Ok(req) => {
                let (progress_tx, mut progress_rx) = mpsc::channel::<String>(32);
                let execution = execute_skill(req, progress_tx);
                tokio::pin!(execution);
                let response = loop {
                    tokio::select! {
                        response = &mut execution => break response,
                        Some(frame) = progress_rx.recv() => {
                            stdout.write_all(frame.as_bytes()).await?;
                            stdout.write_all(b"\n").await?;
                            stdout.flush().await?;
                        }
                    }
                };
                while let Ok(frame) = progress_rx.try_recv() {
                    stdout.write_all(frame.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                }
                response
            }
            Err(err) => SkillResponse {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                error_code: Some("invalid_input".to_string()),
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

async fn execute_skill(req: SkillRequest, progress_tx: mpsc::Sender<String>) -> SkillResponse {
    let configured_timeout_limit = match configured_timeout_limit_from_env() {
        Ok(limit) => limit,
        Err(detail) => {
            return SkillResponse {
                request_id: req.request_id,
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                error_code: Some("runner_timeout_config_invalid".to_string()),
                platform: Some(std::env::consts::OS.to_string()),
                exit_code: None,
                validation: None,
                extra: Some(serde_json::json!({
                    "error_code": "runner_timeout_config_invalid",
                    "message_key": "skill_runner.timeout_config_invalid",
                    "retryable": false,
                })),
                error_text: Some(detail),
            };
        }
    };

    let mut child_launch = match resolve_child_launch(&req) {
        Ok(launch) => launch,
        Err(err) => {
            return SkillResponse {
                request_id: req.request_id,
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                error_code: Some("runner_resolution_failed".to_string()),
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

    let Some(binding) = child_launch.execution_binding.as_mut() else {
        return SkillResponse {
            request_id: req.request_id,
            status: "error".to_string(),
            text: String::new(),
            buttons: None,
            error_code: Some("runner_execution_binding_missing".to_string()),
            platform: Some(std::env::consts::OS.to_string()),
            exit_code: None,
            validation: None,
            extra: Some(serde_json::json!({
                "error_code": "runner_execution_binding_missing",
                "message_key": "skill_runner.execution_binding_missing",
                "retryable": false,
            })),
            error_text: Some("installed launch did not produce an execution binding".to_string()),
        };
    };
    binding.registry_generation = req.expected_registry_generation;
    binding.registry_generation_digest = req.expected_registry_generation_digest.clone();
    binding.base_registry_digest = req.expected_base_registry_digest.clone();
    binding.overlay_generation_digest = req.expected_overlay_generation_digest.clone();
    binding.policy_digest = req.expected_policy_digest.clone();
    binding.admission_receipt_digest = req.expected_admission_receipt_digest.clone();

    let child_req = serde_json::json!({
        "request_id": req.request_id,
        "args": req.args,
        "context": req.context,
        "user_id": req.user_id,
        "chat_id": req.chat_id,
        "user_key": req.user_key,
    });

    let timeout_seconds =
        effective_timeout_seconds(child_launch.timeout_seconds, configured_timeout_limit);
    child_launch.timeout_seconds = timeout_seconds;
    let timeout = Duration::from_secs(timeout_seconds);
    let execution = if child_launch.launcher == LauncherKind::HttpJson {
        run_http_json_skill(&child_launch, &child_req, timeout)
            .await
            .map(|final_output| ChildExecution {
                final_output,
                progress_diagnostics: ProgressFrameDiagnostics::default(),
            })
    } else {
        run_child_skill_streaming(
            &child_launch,
            &child_req.to_string(),
            &req.request_id,
            timeout,
            Some(progress_tx),
        )
        .await
    };
    let response = match execution {
        Ok(execution) => {
            let parsed = parse_child_response(
                &execution.final_output,
                &req.request_id,
                child_launch.strict_protocol,
            );
            match parsed {
                Ok(v) => attach_progress_diagnostics(
                    SkillResponse {
                        request_id: v.request_id.unwrap_or_else(|| "unknown".to_string()),
                        status: v.status.unwrap_or_else(|| "ok".to_string()),
                        text: v.text.unwrap_or_default(),
                        buttons: v.buttons,
                        error_code: v.error_code,
                        platform: v.platform,
                        exit_code: v.exit_code,
                        validation: v.validation,
                        extra: v.extra,
                        error_text: v.error_text,
                    },
                    &execution.progress_diagnostics,
                ),
                Err(err) => attach_progress_diagnostics(
                    SkillResponse {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        buttons: None,
                        error_code: Some(err.error_code.clone()),
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
                            execution.final_output.len()
                        )),
                    },
                    &execution.progress_diagnostics,
                ),
            }
        }
        Err(err) => {
            let diagnostics = err.progress_diagnostics.clone();
            let response = SkillResponse {
                request_id: req.request_id,
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                error_code: Some(err.error_code.to_string()),
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
            };
            match diagnostics.as_ref() {
                Some(value) => attach_progress_diagnostics(response, value),
                None => response,
            }
        }
    };
    attach_execution_binding(response, child_launch.execution_binding.as_ref())
}

fn attach_progress_diagnostics(
    mut response: SkillResponse,
    diagnostics: &ProgressFrameDiagnostics,
) -> SkillResponse {
    if !diagnostics.declared {
        return response;
    }
    let mut extra = match response.extra.take() {
        Some(Value::Object(extra)) => extra,
        Some(child_extra) => serde_json::Map::from_iter([("child_extra".to_string(), child_extra)]),
        None => serde_json::Map::new(),
    };
    extra.insert(
        "progress_frame_diagnostics".to_string(),
        serde_json::to_value(diagnostics).expect("progress diagnostics serialize"),
    );
    response.extra = Some(Value::Object(extra));
    response
}

fn attach_execution_binding(
    mut response: SkillResponse,
    binding: Option<&ExecutionBinding>,
) -> SkillResponse {
    let Some(binding) = binding else {
        return response;
    };
    let mut extra = match response.extra.take() {
        Some(Value::Object(extra)) => extra,
        Some(child_extra) => serde_json::Map::from_iter([("child_extra".to_string(), child_extra)]),
        None => serde_json::Map::new(),
    };
    extra.insert(
        "execution_binding".to_string(),
        serde_json::to_value(binding).expect("execution binding is serializable"),
    );
    response.extra = Some(Value::Object(extra));
    response
}

fn configured_timeout_limit_from_env() -> Result<Option<u64>, String> {
    match std::env::var("SKILL_TIMEOUT_SECONDS") {
        Ok(raw) => parse_configured_timeout_limit(Some(&raw)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("SKILL_TIMEOUT_SECONDS must be valid UTF-8".to_string())
        }
    }
}

fn parse_configured_timeout_limit(raw: Option<&str>) -> Result<Option<u64>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value = raw.parse::<u64>().map_err(|_| {
        "SKILL_TIMEOUT_SECONDS must be a positive integer no greater than 86400".to_string()
    })?;
    if !(1..=86_400).contains(&value) {
        return Err(
            "SKILL_TIMEOUT_SECONDS must be a positive integer no greater than 86400".to_string(),
        );
    }
    Ok(Some(value))
}

fn effective_timeout_seconds(manifest_timeout_seconds: u64, configured_limit: Option<u64>) -> u64 {
    configured_limit
        .map(|limit| limit.min(manifest_timeout_seconds))
        .unwrap_or(manifest_timeout_seconds)
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
    progress_frames: bool,
    installed: bool,
    sandbox_profile: SandboxProfile,
    execution_binding: Option<ExecutionBinding>,
}

const RUNTIME_CHILD_ENV_ALLOWLIST: [&str; 5] = [
    // Credentials arrive as short-lived references.  The child must see the
    // broker-owned token directory in order to redeem an allowed credential
    // environment variable; package manifests should not need to declare
    // this runtime implementation detail themselves.
    "APP_SECRET_TOKEN_DIR",
    "APP_UNRESTRICTED_ADMIN",
    "APP_ALLOW_PATH_OUTSIDE_WORKSPACE",
    "APP_ALLOW_SUDO",
    "APP_WORKSPACE_STATE_DIR",
];

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
            progress_frames: false,
            installed: false,
            sandbox_profile: SandboxProfile::Required,
            execution_binding: None,
        }
    }

    fn installed(spec: SkillLaunchSpec) -> Self {
        let execution_binding = ExecutionBinding {
            skill_name: spec.skill_name.clone(),
            version: spec.version.clone(),
            manifest_digest: spec.manifest_digest.clone(),
            receipt_digest: spec.receipt_digest.clone(),
            registry_generation: 0,
            registry_generation_digest: None,
            base_registry_digest: None,
            overlay_generation_digest: None,
            policy_digest: None,
            admission_receipt_digest: None,
        };
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
            progress_frames: spec.progress_frames,
            installed: true,
            sandbox_profile: spec.sandbox_profile,
            execution_binding: Some(execution_binding),
        }
    }
}

fn resolve_child_launch(request: &SkillRequest) -> Result<ChildLaunch, String> {
    let package_root = std::env::var_os("APP_SKILL_PACKAGES_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/skill-packages"));
    SkillRuntimeResolver::new(&package_root)
        .resolve_pinned(
            &request.skill_name,
            &request.expected_skill_version,
            &request.expected_manifest_digest,
            &request.expected_receipt_digest,
        )
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
        error_code: response.error_code.or_else(|| {
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

#[cfg(test)]
async fn run_child_skill(
    launch: &ChildLaunch,
    input_line: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ExecutionFailure> {
    run_child_skill_streaming(launch, input_line, "ignored", timeout, None)
        .await
        .map(|execution| execution.final_output)
}

async fn run_child_skill_streaming(
    launch: &ChildLaunch,
    input_line: &str,
    expected_request_id: &str,
    timeout: Duration,
    progress_tx: Option<mpsc::Sender<String>>,
) -> Result<ChildExecution, ExecutionFailure> {
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
    let stdout_reader = if launch.progress_frames {
        let expected_request_id = expected_request_id.to_string();
        tokio::spawn(async move {
            read_framed_child_output(stdout, &expected_request_id, progress_tx).await
        })
    } else {
        tokio::spawn(async move {
            read_bounded_output(stdout)
                .await
                .map(|final_output| ChildExecution {
                    final_output,
                    progress_diagnostics: ProgressFrameDiagnostics::default(),
                })
        })
    };
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
            if launch.progress_frames {
                failure.progress_diagnostics = Some(ProgressFrameDiagnostics::declared());
            }
            return Err(failure);
        }
    };
    let execution = stdout_reader
        .await
        .map_err(|error| ExecutionFailure::new("child_stdout_read_failed", error.to_string()))?;
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
        let diagnostics = match &execution {
            Ok(execution) => Some(execution.progress_diagnostics.clone()),
            Err(error) => error.progress_diagnostics.clone(),
        };
        if diagnostics.as_ref().is_some_and(|value| value.declared) {
            failure.progress_diagnostics = diagnostics;
        }
        return Err(failure);
    }
    let execution = execution?;

    if execution.final_output.is_empty() {
        return Err(ExecutionFailure::new(
            "child_stdout_empty",
            "child stdout is empty",
        ));
    }
    Ok(execution)
}

async fn read_framed_child_output(
    stdout: impl tokio::io::AsyncRead + Unpin,
    expected_request_id: &str,
    progress_tx: Option<mpsc::Sender<String>>,
) -> Result<ChildExecution, ExecutionFailure> {
    let mut records = FramedRead::new(
        stdout,
        LinesCodec::new_with_max_length(MAX_PROTOCOL_LINE_BYTES),
    );
    let mut diagnostics = ProgressFrameDiagnostics::declared();
    let mut final_output: Option<Vec<u8>> = None;
    let mut last_sequence = 0_u64;
    let mut accepted_times = VecDeque::<Instant>::new();

    while let Some(record) = records.next().await {
        let line = match record {
            Ok(line) => line,
            Err(LinesCodecError::MaxLineLengthExceeded) => {
                diagnostics.observed = diagnostics.observed.saturating_add(1);
                diagnostics.dropped_oversized = diagnostics.dropped_oversized.saturating_add(1);
                continue;
            }
            Err(LinesCodecError::Io(error)) if error.kind() == std::io::ErrorKind::InvalidData => {
                diagnostics.observed = diagnostics.observed.saturating_add(1);
                diagnostics.dropped_invalid = diagnostics.dropped_invalid.saturating_add(1);
                continue;
            }
            Err(LinesCodecError::Io(error)) => {
                let mut failure =
                    ExecutionFailure::new("child_stdout_read_failed", error.to_string());
                failure.progress_diagnostics = Some(diagnostics);
                return Err(failure);
            }
        };

        if final_output.is_none()
            && validate_response_line(line.as_bytes(), expected_request_id).is_ok()
        {
            let mut output = line.into_bytes();
            output.push(b'\n');
            final_output = Some(output);
            continue;
        }

        diagnostics.observed = diagnostics.observed.saturating_add(1);
        if diagnostics.observed > MAX_PROGRESS_FRAMES_PER_INVOCATION {
            diagnostics.dropped_total_limited = diagnostics.dropped_total_limited.saturating_add(1);
            continue;
        }
        let frame = match validate_progress_frame_line(line.as_bytes(), expected_request_id) {
            Ok(frame) => frame,
            Err(error) => {
                diagnostics.dropped_invalid = diagnostics.dropped_invalid.saturating_add(1);
                if error.code == "progress_frame_request_id_mismatch" {
                    diagnostics.dropped_request_id_mismatch =
                        diagnostics.dropped_request_id_mismatch.saturating_add(1);
                }
                if error.code == "progress_frame_oversized" {
                    diagnostics.dropped_oversized = diagnostics.dropped_oversized.saturating_add(1);
                }
                continue;
            }
        };
        if final_output.is_some() {
            diagnostics.dropped_invalid = diagnostics.dropped_invalid.saturating_add(1);
            continue;
        }
        if frame.sequence <= last_sequence {
            diagnostics.dropped_sequence_invalid =
                diagnostics.dropped_sequence_invalid.saturating_add(1);
            continue;
        }
        last_sequence = frame.sequence;

        let now = Instant::now();
        while accepted_times
            .front()
            .is_some_and(|accepted| now.duration_since(*accepted) >= Duration::from_secs(1))
        {
            accepted_times.pop_front();
        }
        if accepted_times.len() >= MAX_PROGRESS_FRAMES_PER_SECOND {
            diagnostics.dropped_rate_limited = diagnostics.dropped_rate_limited.saturating_add(1);
            continue;
        }
        accepted_times.push_back(now);
        diagnostics.accepted = diagnostics.accepted.saturating_add(1);
        if let Some(sender) = progress_tx.as_ref() {
            let _ = sender.send(line).await;
        }
    }

    let Some(final_output) = final_output else {
        let mut failure = ExecutionFailure::new(
            "child_final_response_missing",
            "progress-capable child exited without a valid final response",
        );
        failure.progress_diagnostics = Some(diagnostics);
        return Err(failure);
    };
    Ok(ChildExecution {
        final_output,
        progress_diagnostics: diagnostics,
    })
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
    for key in RUNTIME_CHILD_ENV_ALLOWLIST {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    // The runner owns the final deadline. Propagate the effective value after
    // inherited environment handling so the child cannot observe a looser cap.
    command.env("SKILL_TIMEOUT_SECONDS", launch.timeout_seconds.to_string());
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
        .take((skill_sdk::MAX_PROTOCOL_LINE_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .await
        .map_err(|error| ExecutionFailure::new("child_output_read_failed", error.to_string()))?;
    if output.len() > skill_sdk::MAX_PROTOCOL_LINE_BYTES {
        let mut failure = ExecutionFailure::new(
            "child_output_truncated",
            format!(
                "child output exceeds {} bytes",
                skill_sdk::MAX_PROTOCOL_LINE_BYTES
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
        .is_some_and(|size| size > skill_sdk::MAX_PROTOCOL_LINE_BYTES as u64)
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
        if output.len().saturating_add(chunk.len()) > skill_sdk::MAX_PROTOCOL_LINE_BYTES {
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
    environment_flag_value_is_enabled(std::env::var("APP_UNRESTRICTED_ADMIN").ok().as_deref())
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

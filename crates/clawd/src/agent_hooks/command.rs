use claw_core::config::ToolSandboxMode;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use super::{
    evaluate_pre_tool_use, merge_hook_decision, normalize_machine_token, HookEvaluation,
    HookPolicy, HookStage, HOOK_EVENT_SCHEMA_VERSION,
};
use crate::{policy_decision::PolicyDecision, AppState};

const HOOK_OUTPUT_SCHEMA_VERSION: u16 = 1;
const MAX_HOOK_FILE_BYTES: u64 = 1024 * 1024;
const MAX_HOOK_TIMEOUT_MS: u64 = 30_000;
const MAX_HOOK_INPUT_BYTES: usize = 256 * 1024;
const MAX_HOOK_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct HookHandlerConfig {
    pub(super) id: String,
    pub(super) stage: String,
    pub(super) kind: String,
    pub(super) enabled: bool,
    pub(super) trusted: bool,
    pub(super) blocking: bool,
    pub(super) path: String,
    pub(super) args: Vec<String>,
    pub(super) content_sha256: String,
    pub(super) timeout_ms: u64,
    pub(super) max_input_bytes: usize,
    pub(super) max_output_bytes: usize,
    pub(super) max_attempts: u8,
    pub(super) failure_policy: String,
}

impl Default for HookHandlerConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            stage: String::new(),
            kind: String::new(),
            enabled: false,
            trusted: false,
            blocking: true,
            path: String::new(),
            args: Vec::new(),
            content_sha256: String::new(),
            timeout_ms: 3_000,
            max_input_bytes: 32 * 1024,
            max_output_bytes: 16 * 1024,
            max_attempts: 1,
            failure_policy: "deny".to_string(),
        }
    }
}

#[derive(Debug)]
struct LoadedHookConfiguration {
    policy: HookPolicy,
    handlers: Vec<HookHandlerConfig>,
    error_code: Option<&'static str>,
}

#[derive(Debug)]
pub(super) struct ValidatedCommandHandler {
    id: String,
    stage: HookStage,
    blocking: bool,
    path: PathBuf,
    args: Vec<String>,
    content_sha256: String,
    timeout: Duration,
    max_input_bytes: usize,
    max_output_bytes: usize,
    failure_policy: HookFailurePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookFailurePolicy {
    Deny,
    Ignore,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HookHandlerOutput {
    schema_version: u16,
    decision: String,
    reason_code: String,
    #[serde(default)]
    status_code: Option<String>,
}

#[derive(Debug)]
pub(super) struct HandlerRunResult {
    pub(super) decision: PolicyDecision,
    pub(super) reason_code: String,
    pub(super) status: &'static str,
    pub(super) error_code: Option<&'static str>,
    pub(super) duration_ms: u64,
    pub(super) output_truncated: bool,
}

impl HandlerRunResult {
    fn failure(decision: PolicyDecision, error_code: &'static str, duration_ms: u64) -> Self {
        Self {
            decision,
            reason_code: error_code.to_string(),
            status: "error",
            error_code: Some(error_code),
            duration_ms,
            output_truncated: false,
        }
    }
}

pub(crate) async fn pre_tool_use_outcome_for_state(
    state: &AppState,
    task_id: &str,
    tool_or_skill: &str,
    args: &Value,
) -> HookEvaluation {
    let action_ref = super::tool_action_ref(tool_or_skill, args);
    let loaded = load_hook_configuration(state);
    let mut outcome = evaluate_pre_tool_use(&loaded.policy, &action_ref);
    let mut handler_observations = Vec::new();
    if let Some(error_code) = loaded.error_code {
        merge_hook_decision(
            &mut outcome,
            PolicyDecision::Deny,
            "hook_config_invalid".to_string(),
        );
        handler_observations.push(handler_observation(
            "hook_config",
            "configuration",
            HookStage::PreToolUse,
            &action_ref,
            &HandlerRunResult::failure(PolicyDecision::Deny, error_code, 0),
            true,
            "invalid",
            None,
        ));
        return HookEvaluation {
            outcome,
            handler_observations,
        };
    }
    if outcome.decision_kind() == Some(PolicyDecision::Deny) {
        return HookEvaluation {
            outcome,
            handler_observations,
        };
    }
    let event = pre_tool_hook_event(task_id, tool_or_skill, args, &action_ref);
    let cancellation = state
        .worker
        .task_cancellation_token(task_id)
        .unwrap_or_default();
    for handler in loaded
        .handlers
        .into_iter()
        .filter(|handler| handler.enabled)
    {
        let validated = match validate_command_handler(&state.skill_rt.workspace_root, handler) {
            Ok(handler) => handler,
            Err((handler_id, error_code)) => {
                let result = HandlerRunResult::failure(PolicyDecision::Deny, error_code, 0);
                merge_hook_decision(&mut outcome, result.decision, result.reason_code.clone());
                handler_observations.push(handler_observation(
                    &handler_id,
                    "command",
                    HookStage::PreToolUse,
                    &action_ref,
                    &result,
                    true,
                    "invalid",
                    None,
                ));
                continue;
            }
        };
        if validated.stage != HookStage::PreToolUse {
            continue;
        }
        let result = execute_command_handler(
            &validated,
            &state.skill_rt.workspace_root,
            &event,
            cancellation.clone(),
            ToolSandboxMode::ReadOnly,
        )
        .await;
        merge_hook_decision(&mut outcome, result.decision, result.reason_code.clone());
        handler_observations.push(handler_observation(
            &validated.id,
            "command",
            validated.stage,
            &action_ref,
            &result,
            validated.blocking,
            "trusted",
            Some(&validated.content_sha256),
        ));
    }
    HookEvaluation {
        outcome,
        handler_observations,
    }
}

fn load_hook_configuration(state: &AppState) -> LoadedHookConfiguration {
    let path = state
        .skill_rt
        .workspace_root
        .join("configs/agent_guard.toml");
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadedHookConfiguration {
                policy: HookPolicy::default(),
                handlers: Vec::new(),
                error_code: None,
            };
        }
        Err(_) => {
            return LoadedHookConfiguration {
                policy: HookPolicy::default(),
                handlers: Vec::new(),
                error_code: Some("hook_config_read_failed"),
            };
        }
    };
    let root = match toml::from_str::<toml::Value>(&raw) {
        Ok(root) => root,
        Err(_) => {
            return LoadedHookConfiguration {
                policy: HookPolicy::default(),
                handlers: Vec::new(),
                error_code: Some("hook_config_parse_failed"),
            };
        }
    };
    let policy = HookPolicy {
        blocked_action_refs: toml_string_array(&root, &["agent", "hooks", "blocked_action_refs"]),
        blocked_tools: toml_string_array(&root, &["agent", "hooks", "blocked_tools"]),
        require_confirmation_action_refs: toml_string_array(
            &root,
            &["agent", "hooks", "require_confirmation_action_refs"],
        ),
        background_wait_action_refs: toml_string_array(
            &root,
            &["agent", "hooks", "background_wait_action_refs"],
        ),
    };
    let handler_values = root
        .get("agent")
        .and_then(|value| value.get("hooks"))
        .and_then(|value| value.get("handlers"))
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut handlers = Vec::with_capacity(handler_values.len());
    let mut ids = std::collections::BTreeSet::new();
    for value in handler_values {
        let handler = match value.try_into::<HookHandlerConfig>() {
            Ok(handler) => handler,
            Err(_) => {
                return LoadedHookConfiguration {
                    policy,
                    handlers: Vec::new(),
                    error_code: Some("hook_handler_config_invalid"),
                };
            }
        };
        if handler.enabled && !ids.insert(handler.id.trim().to_string()) {
            return LoadedHookConfiguration {
                policy,
                handlers: Vec::new(),
                error_code: Some("hook_handler_id_duplicate"),
            };
        }
        handlers.push(handler);
    }
    LoadedHookConfiguration {
        policy,
        handlers,
        error_code: None,
    }
}

pub(super) fn validate_command_handler(
    workspace_root: &Path,
    handler: HookHandlerConfig,
) -> Result<ValidatedCommandHandler, (String, &'static str)> {
    let handler_id = safe_handler_id(&handler.id);
    if !is_machine_token(handler.id.trim(), 64) {
        return Err((handler_id, "hook_handler_id_invalid"));
    }
    if handler.kind.trim() != "command" {
        return Err((handler_id, "hook_handler_kind_unsupported"));
    }
    let Some(stage) = HookStage::parse_token(&handler.stage) else {
        return Err((handler_id, "hook_handler_stage_invalid"));
    };
    if stage == HookStage::PreToolUse && !handler.blocking {
        return Err((handler_id, "hook_async_decision_handler_invalid"));
    }
    if !handler.trusted {
        return Err((handler_id, "hook_handler_untrusted"));
    }
    if handler.timeout_ms == 0 || handler.timeout_ms > MAX_HOOK_TIMEOUT_MS {
        return Err((handler_id, "hook_handler_timeout_invalid"));
    }
    if handler.max_input_bytes == 0 || handler.max_input_bytes > MAX_HOOK_INPUT_BYTES {
        return Err((handler_id, "hook_handler_input_limit_invalid"));
    }
    if handler.max_output_bytes == 0 || handler.max_output_bytes > MAX_HOOK_OUTPUT_BYTES {
        return Err((handler_id, "hook_handler_output_limit_invalid"));
    }
    if handler.max_attempts != 1 {
        return Err((handler_id, "hook_handler_attempt_limit_invalid"));
    }
    if handler.args.len() > 32 || handler.args.iter().any(|arg| arg.len() > 1024) {
        return Err((handler_id, "hook_handler_args_invalid"));
    }
    let failure_policy = match handler.failure_policy.trim() {
        "deny" => HookFailurePolicy::Deny,
        "ignore" => HookFailurePolicy::Ignore,
        _ => return Err((handler_id, "hook_handler_failure_policy_invalid")),
    };
    let relative_path = Path::new(handler.path.trim());
    if relative_path.as_os_str().is_empty() || relative_path.is_absolute() {
        return Err((handler_id, "hook_handler_path_invalid"));
    }
    let workspace = workspace_root
        .canonicalize()
        .map_err(|_| (handler_id.clone(), "hook_workspace_unavailable"))?;
    let path = workspace_root
        .join(relative_path)
        .canonicalize()
        .map_err(|_| (handler_id.clone(), "hook_handler_path_unavailable"))?;
    if !path.starts_with(&workspace) || !path.is_file() {
        return Err((handler_id, "hook_handler_path_outside_workspace"));
    }
    let metadata = std::fs::metadata(&path)
        .map_err(|_| (handler_id.clone(), "hook_handler_metadata_failed"))?;
    if metadata.len() == 0 || metadata.len() > MAX_HOOK_FILE_BYTES {
        return Err((handler_id, "hook_handler_file_size_invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err((handler_id, "hook_handler_not_executable"));
        }
    }
    let expected_hash = handler.content_sha256.trim();
    if !valid_sha256_label(expected_hash) {
        return Err((handler_id, "hook_handler_hash_invalid"));
    }
    let bytes =
        std::fs::read(&path).map_err(|_| (handler_id.clone(), "hook_handler_read_failed"))?;
    let actual_hash = format!("sha256:{:x}", Sha256::digest(bytes));
    if actual_hash != expected_hash {
        return Err((handler_id, "hook_handler_hash_mismatch"));
    }
    Ok(ValidatedCommandHandler {
        id: handler.id,
        stage,
        blocking: handler.blocking,
        path,
        args: handler.args,
        content_sha256: actual_hash,
        timeout: Duration::from_millis(handler.timeout_ms),
        max_input_bytes: handler.max_input_bytes,
        max_output_bytes: handler.max_output_bytes,
        failure_policy,
    })
}

pub(super) async fn execute_command_handler(
    handler: &ValidatedCommandHandler,
    workspace_root: &Path,
    event: &Value,
    cancellation: CancellationToken,
    sandbox_mode: ToolSandboxMode,
) -> HandlerRunResult {
    let started = Instant::now();
    let mut input = match serde_json::to_vec(event) {
        Ok(input) => input,
        Err(_) => return handler_failure(handler, "hook_event_encode_failed", started, false),
    };
    input.push(b'\n');
    if input.len() > handler.max_input_bytes {
        return handler_failure(handler, "hook_event_too_large", started, false);
    }
    let prepared = crate::process_sandbox::prepare_process_command(
        &handler.path,
        crate::process_sandbox::ProcessSandboxRequest {
            mode: sandbox_mode,
            workspace_root,
            execution_root: workspace_root,
            network: crate::process_sandbox::ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
        },
    );
    let mut command = match prepared {
        Ok(prepared) => prepared.command,
        Err(error_code) => return handler_failure(handler, error_code, started, false),
    };
    command
        .args(&handler.args)
        .current_dir(workspace_root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "RUSTCLAW_HOOK_SCHEMA_VERSION",
            HOOK_EVENT_SCHEMA_VERSION.to_string(),
        )
        .env("RUSTCLAW_HOOK_ID", &handler.id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return handler_failure(handler, "hook_handler_spawn_failed", started, false),
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let output_limit = handler.max_output_bytes + 1;
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(stdout) = stdout {
            let _ = stdout
                .take(output_limit as u64)
                .read_to_end(&mut bytes)
                .await;
        }
        bytes
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(stderr) = stderr {
            let _ = stderr
                .take(output_limit as u64)
                .read_to_end(&mut bytes)
                .await;
        }
        bytes
    });
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill().await;
        return handler_failure(handler, "hook_handler_stdin_unavailable", started, false);
    };
    let deadline = tokio::time::Instant::now() + handler.timeout;
    let write_result = tokio::select! {
        _ = cancellation.cancelled() => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return handler_failure(handler, "hook_handler_cancelled", started, false);
        }
        result = tokio::time::timeout_at(deadline, async {
            stdin.write_all(&input).await?;
            stdin.shutdown().await
        }) => result,
    };
    match write_result {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return handler_failure(handler, "hook_handler_input_failed", started, false);
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return handler_failure(handler, "hook_handler_timeout", started, false);
        }
    }
    let status = tokio::select! {
        _ = cancellation.cancelled() => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return handler_failure(handler, "hook_handler_cancelled", started, false);
        }
        result = tokio::time::timeout_at(deadline, child.wait()) => {
            match result {
                Ok(Ok(status)) => status,
                Ok(Err(_)) => return handler_failure(handler, "hook_handler_wait_failed", started, false),
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return handler_failure(handler, "hook_handler_timeout", started, false);
                }
            }
        }
    };
    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();
    let output_truncated =
        stdout.len() > handler.max_output_bytes || stderr.len() > handler.max_output_bytes;
    if output_truncated {
        return handler_failure(handler, "hook_handler_output_too_large", started, true);
    }
    if !status.success() {
        return handler_failure(handler, "hook_handler_exit_nonzero", started, false);
    }
    let output = match parse_handler_output(&stdout, handler.blocking) {
        Ok(output) => output,
        Err(error_code) => return handler_failure(handler, error_code, started, false),
    };
    HandlerRunResult {
        decision: output.0,
        reason_code: output.1,
        status: "ok",
        error_code: None,
        duration_ms: elapsed_ms(started),
        output_truncated: false,
    }
}

pub(super) fn parse_handler_output(
    stdout: &[u8],
    blocking: bool,
) -> Result<(PolicyDecision, String), &'static str> {
    let output = std::str::from_utf8(stdout).map_err(|_| "hook_handler_output_utf8_invalid")?;
    let output = output.trim_end_matches(['\r', '\n']);
    if output.is_empty() || output.contains(['\r', '\n']) {
        return Err("hook_handler_output_line_invalid");
    }
    let output: HookHandlerOutput =
        serde_json::from_str(output).map_err(|_| "hook_handler_output_schema_invalid")?;
    if output.schema_version != HOOK_OUTPUT_SCHEMA_VERSION {
        return Err("hook_handler_output_version_unsupported");
    }
    let decision = PolicyDecision::parse_token(&output.decision)
        .ok_or("hook_handler_output_decision_invalid")?;
    if !blocking && decision != PolicyDecision::Allow {
        return Err("hook_async_decision_forbidden");
    }
    if !is_machine_token(&output.reason_code, 128)
        || output
            .status_code
            .as_deref()
            .is_some_and(|value| !is_machine_token(value, 128))
    {
        return Err("hook_handler_output_token_invalid");
    }
    Ok((decision, output.reason_code))
}

fn handler_failure(
    handler: &ValidatedCommandHandler,
    error_code: &'static str,
    started: Instant,
    output_truncated: bool,
) -> HandlerRunResult {
    let decision = match handler.failure_policy {
        HookFailurePolicy::Deny => PolicyDecision::Deny,
        HookFailurePolicy::Ignore => PolicyDecision::Allow,
    };
    let mut result = HandlerRunResult::failure(decision, error_code, elapsed_ms(started));
    result.output_truncated = output_truncated;
    result
}

pub(super) fn pre_tool_hook_event(
    task_id: &str,
    tool_or_skill: &str,
    args: &Value,
    action_ref: &str,
) -> Value {
    let mut argument_fields = args
        .as_object()
        .into_iter()
        .flat_map(|object| object.keys())
        .filter(|key| is_machine_token(key, 128))
        .cloned()
        .collect::<Vec<_>>();
    argument_fields.sort();
    json!({
        "schema_version": HOOK_EVENT_SCHEMA_VERSION,
        "event_type": HookStage::PreToolUse.as_token(),
        "task_id": task_id,
        "action_ref": action_ref,
        "tool_or_skill": normalize_machine_token(tool_or_skill),
        "argument_count": args.as_object().map(|object| object.len()).unwrap_or(0),
        "argument_byte_count": args.to_string().len(),
        "argument_fields": argument_fields,
    })
}

fn handler_observation(
    handler_id: &str,
    handler_kind: &str,
    stage: HookStage,
    action_ref: &str,
    result: &HandlerRunResult,
    blocking: bool,
    trust_status: &str,
    content_sha256: Option<&str>,
) -> Value {
    json!({
        "schema_version": 1,
        "event_schema_version": HOOK_EVENT_SCHEMA_VERSION,
        "event_type": stage.as_token(),
        "owner_layer": "agent_hooks",
        "stage": stage.as_token(),
        "action_ref": action_ref,
        "handler_id": handler_id,
        "handler_kind": handler_kind,
        "blocking": blocking,
        "trust_status": trust_status,
        "content_sha256": content_sha256,
        "decision": result.decision.as_token(),
        "reason_code": result.reason_code,
        "status_code": result.reason_code,
        "status": result.status,
        "error_code": result.error_code,
        "duration_ms": result.duration_ms,
        "attempts": 1,
        "output_truncated": result.output_truncated,
    })
}

fn toml_string_array(root: &toml::Value, path: &[&str]) -> Vec<String> {
    let mut cursor = root;
    for segment in path {
        let Some(next) = cursor.get(*segment) else {
            return Vec::new();
        };
        cursor = next;
    }
    cursor
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .map(normalize_machine_token)
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn safe_handler_id(value: &str) -> String {
    let value = value.trim();
    is_machine_token(value, 64)
        .then(|| value.to_string())
        .unwrap_or_else(|| "hook_handler_invalid".to_string())
}

fn is_machine_token(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
}

fn valid_sha256_label(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

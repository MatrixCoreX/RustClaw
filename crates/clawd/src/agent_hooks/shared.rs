use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::{normalize_machine_token, HookPolicy, HookStage, HOOK_EVENT_SCHEMA_VERSION};
use crate::policy_decision::PolicyDecision;

pub(super) const HOOK_OUTPUT_SCHEMA_VERSION: u16 = 1;
pub(super) const MAX_HOOK_TIMEOUT_MS: u64 = 30_000;
pub(super) const MAX_HOOK_INPUT_BYTES: usize = 256 * 1024;
pub(super) const MAX_HOOK_OUTPUT_BYTES: usize = 64 * 1024;

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
    pub(super) url: String,
    pub(super) auth_token_env: Option<String>,
    pub(super) allow_insecure_loopback: bool,
    pub(super) capability: String,
    pub(super) event_argument: String,
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
            url: String::new(),
            auth_token_env: None,
            allow_insecure_loopback: false,
            capability: String::new(),
            event_argument: "hook_event".to_string(),
            timeout_ms: 3_000,
            max_input_bytes: 32 * 1024,
            max_output_bytes: 16 * 1024,
            max_attempts: 1,
            failure_policy: "deny".to_string(),
        }
    }
}

#[derive(Debug)]
pub(super) struct LoadedHookConfiguration {
    pub(super) policy: HookPolicy,
    pub(super) handlers: Vec<HookHandlerConfig>,
    pub(super) error_code: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub(super) struct ValidatedHookHandler {
    pub(super) id: String,
    pub(super) stage: HookStage,
    pub(super) blocking: bool,
    pub(super) timeout: Duration,
    pub(super) max_input_bytes: usize,
    pub(super) max_output_bytes: usize,
    pub(super) max_attempts: u8,
    pub(super) failure_policy: HookFailurePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HookFailurePolicy {
    Deny,
    Ignore,
}

impl HookFailurePolicy {
    pub(super) fn as_token(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Ignore => "ignore",
        }
    }
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
    pub(super) attempts: u8,
    pub(super) output_truncated: bool,
}

impl HandlerRunResult {
    pub(super) fn validation_failure(error_code: &'static str) -> Self {
        Self {
            decision: PolicyDecision::Deny,
            reason_code: error_code.to_string(),
            status: "error",
            error_code: Some(error_code),
            duration_ms: 0,
            attempts: 0,
            output_truncated: false,
        }
    }
}

#[derive(Debug)]
pub(super) struct ExecutedHook {
    pub(super) handler: ValidatedHookHandler,
    pub(super) handler_kind: &'static str,
    pub(super) trust_status: &'static str,
    pub(super) content_sha256: Option<String>,
    pub(super) result: HandlerRunResult,
}

pub(super) fn validate_common_handler(
    handler: &HookHandlerConfig,
    expected_kind: &'static str,
    supported_attempts: u8,
) -> Result<ValidatedHookHandler, (String, &'static str)> {
    let handler_id = safe_handler_id(&handler.id);
    if !is_machine_token(handler.id.trim(), 64) {
        return Err((handler_id, "hook_handler_id_invalid"));
    }
    if handler.kind.trim() != expected_kind {
        return Err((handler_id, "hook_handler_kind_unsupported"));
    }
    let Some(stage) = HookStage::parse_token(&handler.stage) else {
        return Err((handler_id, "hook_handler_stage_invalid"));
    };
    if handler.blocking && !matches!(stage, HookStage::PreToolUse | HookStage::PermissionRequest) {
        return Err((handler_id, "hook_handler_blocking_stage_invalid"));
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
    if handler.max_attempts == 0 || handler.max_attempts > supported_attempts {
        return Err((handler_id, "hook_handler_attempt_limit_invalid"));
    }
    let failure_policy = match handler.failure_policy.trim() {
        "deny" => HookFailurePolicy::Deny,
        "ignore" => HookFailurePolicy::Ignore,
        _ => return Err((handler_id, "hook_handler_failure_policy_invalid")),
    };
    Ok(ValidatedHookHandler {
        id: handler.id.clone(),
        stage,
        blocking: handler.blocking,
        timeout: Duration::from_millis(handler.timeout_ms),
        max_input_bytes: handler.max_input_bytes,
        max_output_bytes: handler.max_output_bytes,
        max_attempts: handler.max_attempts,
        failure_policy,
    })
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
    let output = serde_json::from_str(output).map_err(|_| "hook_handler_output_schema_invalid")?;
    parse_handler_output_value(output, blocking)
}

pub(super) fn parse_handler_output_value(
    output: Value,
    blocking: bool,
) -> Result<(PolicyDecision, String), &'static str> {
    let output: HookHandlerOutput =
        serde_json::from_value(output).map_err(|_| "hook_handler_output_schema_invalid")?;
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

pub(super) fn handler_failure(
    handler: &ValidatedHookHandler,
    error_code: &'static str,
    started: Instant,
    attempts: u8,
    output_truncated: bool,
) -> HandlerRunResult {
    let decision = match handler.failure_policy {
        HookFailurePolicy::Deny => PolicyDecision::Deny,
        HookFailurePolicy::Ignore => PolicyDecision::Allow,
    };
    HandlerRunResult {
        decision,
        reason_code: error_code.to_string(),
        status: "error",
        error_code: Some(error_code),
        duration_ms: elapsed_ms(started),
        attempts,
        output_truncated,
    }
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

pub(super) fn lifecycle_hook_event(
    stage: HookStage,
    task_id: &str,
    action_ref: &str,
    metadata: Value,
) -> Value {
    let metadata = metadata
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    lifecycle_metadata_key_allowed(key)
                        .then(|| sanitize_lifecycle_metadata_value(value, 0))
                        .flatten()
                        .map(|value| (key.clone(), value))
                })
                .collect::<serde_json::Map<_, _>>()
        })
        .unwrap_or_default();
    json!({
        "schema_version": HOOK_EVENT_SCHEMA_VERSION,
        "event_type": stage.as_token(),
        "task_id": task_id,
        "action_ref": action_ref,
        "metadata": metadata,
    })
}

fn sanitize_lifecycle_metadata_value(value: &Value, depth: usize) -> Option<Value> {
    if depth > 2 {
        return None;
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) if is_machine_token(value, 256) => Some(Value::String(value.clone())),
        Value::Array(values) if values.len() <= 64 => {
            let sanitized = values
                .iter()
                .filter_map(|value| sanitize_lifecycle_metadata_value(value, depth + 1))
                .collect::<Vec<_>>();
            (!sanitized.is_empty() || values.is_empty()).then(|| Value::Array(sanitized))
        }
        Value::Object(object) if object.len() <= 64 => {
            let sanitized = object
                .iter()
                .filter_map(|(key, value)| {
                    lifecycle_metadata_key_allowed(key)
                        .then(|| sanitize_lifecycle_metadata_value(value, depth + 1))
                        .flatten()
                        .map(|value| (key.clone(), value))
                })
                .collect::<serde_json::Map<_, _>>();
            (!sanitized.is_empty() || object.is_empty()).then(|| Value::Object(sanitized))
        }
        _ => None,
    }
}

fn lifecycle_metadata_key_allowed(key: &str) -> bool {
    is_machine_token(key, 128)
        && !matches!(
            key,
            "user_prompt"
                | "user_text"
                | "final_answer"
                | "response_text"
                | "raw_response"
                | "raw_output"
                | "api_key"
        )
        && !key.split(['_', '-', '.']).any(|part| {
            matches!(
                part,
                "secret" | "password" | "credential" | "token" | "api_key"
            )
        })
}

pub(super) fn handler_observation(
    handler_id: &str,
    handler_kind: &str,
    stage: HookStage,
    action_ref: &str,
    result: &HandlerRunResult,
    blocking: bool,
    failure_policy: &str,
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
        "failure_policy": failure_policy,
        "trust_status": trust_status,
        "content_sha256": content_sha256,
        "decision": result.decision.as_token(),
        "reason_code": result.reason_code,
        "status_code": result.reason_code,
        "status": result.status,
        "error_code": result.error_code,
        "duration_ms": result.duration_ms,
        "attempts": result.attempts,
        "output_truncated": result.output_truncated,
    })
}

pub(super) fn safe_handler_id(value: &str) -> String {
    let value = value.trim();
    is_machine_token(value, 64)
        .then(|| value.to_string())
        .unwrap_or_else(|| "hook_handler_invalid".to_string())
}

pub(super) fn is_machine_token(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
}

pub(super) fn is_env_reference(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_uppercase())
        && bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        && value.len() <= 128
}

pub(super) fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

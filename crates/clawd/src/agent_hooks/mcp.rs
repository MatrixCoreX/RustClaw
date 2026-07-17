use serde_json::{Map, Value};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use super::shared::{
    elapsed_ms, handler_failure, is_machine_token, parse_handler_output_value,
    validate_common_handler, ExecutedHook, HandlerRunResult, HookHandlerConfig,
    ValidatedHookHandler,
};
use crate::mcp_runtime::McpRuntime;

#[derive(Debug)]
pub(super) struct ValidatedMcpHandler {
    common: ValidatedHookHandler,
    capability: String,
    event_argument: String,
}

pub(super) async fn run_mcp_handler(
    runtime: &McpRuntime,
    handler: HookHandlerConfig,
    event: &Value,
    cancellation: CancellationToken,
) -> Result<ExecutedHook, (String, &'static str)> {
    let handler = validate_mcp_handler(runtime, handler)?;
    let result = execute_mcp_handler(runtime, &handler, event, cancellation).await;
    Ok(ExecutedHook {
        handler: handler.common,
        handler_kind: "mcp",
        trust_status: "trusted",
        content_sha256: None,
        result,
    })
}

pub(super) fn validate_mcp_handler(
    runtime: &McpRuntime,
    handler: HookHandlerConfig,
) -> Result<ValidatedMcpHandler, (String, &'static str)> {
    let common = validate_common_handler(&handler, "mcp", 1)?;
    let capability = handler.capability.trim();
    if !is_machine_token(capability, 192) {
        return Err((common.id, "hook_mcp_capability_invalid"));
    }
    if !is_machine_token(handler.event_argument.trim(), 64) {
        return Err((common.id, "hook_mcp_event_argument_invalid"));
    }
    let descriptor = runtime
        .tool(capability)
        .ok_or_else(|| (common.id.clone(), "hook_mcp_capability_unavailable"))?;
    let policy = &descriptor.policy;
    if !matches!(policy.effect.as_str(), "observe" | "validate")
        || policy.risk_level != "low"
        || !policy.idempotent
        || policy.filesystem_write
        || policy.external_publish
        || policy.credential_access
        || policy.subprocess
        || policy.package_install
        || policy.privilege_escalation
    {
        return Err((common.id, "hook_mcp_policy_unsafe"));
    }
    Ok(ValidatedMcpHandler {
        common,
        capability: capability.to_string(),
        event_argument: handler.event_argument,
    })
}

pub(super) async fn execute_mcp_handler(
    runtime: &McpRuntime,
    handler: &ValidatedMcpHandler,
    event: &Value,
    cancellation: CancellationToken,
) -> HandlerRunResult {
    let started = Instant::now();
    let event_bytes = match serde_json::to_vec(event) {
        Ok(bytes) if bytes.len() <= handler.common.max_input_bytes => bytes,
        Ok(_) => {
            return handler_failure(&handler.common, "hook_event_too_large", started, 0, false);
        }
        Err(_) => {
            return handler_failure(
                &handler.common,
                "hook_event_encode_failed",
                started,
                0,
                false,
            );
        }
    };
    drop(event_bytes);
    let args = Value::Object(Map::from_iter([(
        handler.event_argument.clone(),
        event.clone(),
    )]));
    let call_cancellation = CancellationToken::new();
    let call = runtime.call(&handler.capability, args, Some(call_cancellation.clone()));
    let outcome = tokio::select! {
        _ = cancellation.cancelled() => {
            call_cancellation.cancel();
            return handler_failure(
                &handler.common,
                "hook_handler_cancelled",
                started,
                1,
                false,
            );
        }
        result = tokio::time::timeout(handler.common.timeout, call) => {
            match result {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => {
                    return handler_failure(
                        &handler.common,
                        error.code(),
                        started,
                        1,
                        false,
                    );
                }
                Err(_) => {
                    call_cancellation.cancel();
                    return handler_failure(
                        &handler.common,
                        "hook_handler_timeout",
                        started,
                        1,
                        false,
                    );
                }
            }
        }
    };
    if outcome.truncated {
        return handler_failure(
            &handler.common,
            "hook_handler_output_too_large",
            started,
            1,
            true,
        );
    }
    if outcome.status != "ok" {
        return handler_failure(&handler.common, "hook_mcp_result_error", started, 1, false);
    }
    let Some(output) = outcome.structured_content else {
        return handler_failure(
            &handler.common,
            "hook_mcp_structured_output_required",
            started,
            1,
            false,
        );
    };
    if serde_json::to_vec(&output)
        .map(|bytes| bytes.len() > handler.common.max_output_bytes)
        .unwrap_or(true)
    {
        return handler_failure(
            &handler.common,
            "hook_handler_output_too_large",
            started,
            1,
            true,
        );
    }
    let output = match parse_handler_output_value(output, handler.common.blocking) {
        Ok(output) => output,
        Err(error_code) => {
            return handler_failure(&handler.common, error_code, started, 1, false);
        }
    };
    HandlerRunResult {
        decision: output.0,
        reason_code: output.1,
        status: "ok",
        error_code: None,
        duration_ms: elapsed_ms(started),
        attempts: 1,
        output_truncated: false,
    }
}

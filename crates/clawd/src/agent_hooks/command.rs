use claw_core::config::ToolSandboxMode;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use super::shared::{
    elapsed_ms, handler_failure, validate_common_handler, ExecutedHook, HandlerRunResult,
    HookHandlerConfig, ValidatedHookHandler,
};
const MAX_HOOK_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub(super) struct ValidatedCommandHandler {
    common: ValidatedHookHandler,
    path: PathBuf,
    args: Vec<String>,
    content_sha256: String,
}

pub(super) async fn run_command_handler(
    workspace_root: &Path,
    handler: HookHandlerConfig,
    event: &Value,
    cancellation: CancellationToken,
) -> Result<ExecutedHook, (String, &'static str)> {
    let handler = validate_command_handler(workspace_root, handler)?;
    let result = execute_command_handler(
        &handler,
        workspace_root,
        event,
        cancellation,
        ToolSandboxMode::ReadOnly,
    )
    .await;
    Ok(ExecutedHook {
        handler: handler.common.clone(),
        handler_kind: "command",
        trust_status: "trusted",
        content_sha256: Some(handler.content_sha256),
        result,
    })
}

pub(super) fn validate_command_handler(
    workspace_root: &Path,
    handler: HookHandlerConfig,
) -> Result<ValidatedCommandHandler, (String, &'static str)> {
    let common = validate_common_handler(&handler, "command", 1)?;
    if handler.args.len() > 32 || handler.args.iter().any(|arg| arg.len() > 1024) {
        return Err((common.id, "hook_handler_args_invalid"));
    }
    let relative_path = Path::new(handler.path.trim());
    if relative_path.as_os_str().is_empty() || relative_path.is_absolute() {
        return Err((common.id, "hook_handler_path_invalid"));
    }
    let workspace = workspace_root
        .canonicalize()
        .map_err(|_| (common.id.clone(), "hook_workspace_unavailable"))?;
    let path = workspace_root
        .join(relative_path)
        .canonicalize()
        .map_err(|_| (common.id.clone(), "hook_handler_path_unavailable"))?;
    if !path.starts_with(&workspace) || !path.is_file() {
        return Err((common.id, "hook_handler_path_outside_workspace"));
    }
    let metadata = std::fs::metadata(&path)
        .map_err(|_| (common.id.clone(), "hook_handler_metadata_failed"))?;
    if metadata.len() == 0 || metadata.len() > MAX_HOOK_FILE_BYTES {
        return Err((common.id, "hook_handler_file_size_invalid"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err((common.id, "hook_handler_not_executable"));
        }
    }
    let expected_hash = handler.content_sha256.trim();
    if !valid_sha256_label(expected_hash) {
        return Err((common.id, "hook_handler_hash_invalid"));
    }
    let bytes =
        std::fs::read(&path).map_err(|_| (common.id.clone(), "hook_handler_read_failed"))?;
    let actual_hash = format!("sha256:{:x}", Sha256::digest(bytes));
    if actual_hash != expected_hash {
        return Err((common.id, "hook_handler_hash_mismatch"));
    }
    Ok(ValidatedCommandHandler {
        common,
        path,
        args: handler.args,
        content_sha256: actual_hash,
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
    input.push(b'\n');
    if input.len() > handler.common.max_input_bytes {
        return handler_failure(&handler.common, "hook_event_too_large", started, 0, false);
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
        Err(error_code) => {
            return handler_failure(&handler.common, error_code, started, 0, false);
        }
    };
    command
        .args(&handler.args)
        .current_dir(workspace_root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "RUSTCLAW_HOOK_SCHEMA_VERSION",
            super::HOOK_EVENT_SCHEMA_VERSION.to_string(),
        )
        .env("RUSTCLAW_HOOK_ID", &handler.common.id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            return handler_failure(
                &handler.common,
                "hook_handler_spawn_failed",
                started,
                1,
                false,
            );
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let output_limit = handler.common.max_output_bytes + 1;
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
        return handler_failure(
            &handler.common,
            "hook_handler_stdin_unavailable",
            started,
            1,
            false,
        );
    };
    let deadline = tokio::time::Instant::now() + handler.common.timeout;
    let write_result = tokio::select! {
        _ = cancellation.cancelled() => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return handler_failure(&handler.common, "hook_handler_cancelled", started, 1, false);
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
            return handler_failure(
                &handler.common,
                "hook_handler_input_failed",
                started,
                1,
                false,
            );
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return handler_failure(&handler.common, "hook_handler_timeout", started, 1, false);
        }
    }
    let status = tokio::select! {
        _ = cancellation.cancelled() => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return handler_failure(&handler.common, "hook_handler_cancelled", started, 1, false);
        }
        result = tokio::time::timeout_at(deadline, child.wait()) => {
            match result {
                Ok(Ok(status)) => status,
                Ok(Err(_)) => return handler_failure(
                    &handler.common,
                    "hook_handler_wait_failed",
                    started,
                    1,
                    false,
                ),
                Err(_) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
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
    let stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();
    let output_truncated = stdout.len() > handler.common.max_output_bytes
        || stderr.len() > handler.common.max_output_bytes;
    if output_truncated {
        return handler_failure(
            &handler.common,
            "hook_handler_output_too_large",
            started,
            1,
            true,
        );
    }
    if !status.success() {
        return handler_failure(
            &handler.common,
            "hook_handler_exit_nonzero",
            started,
            1,
            false,
        );
    }
    let output = match super::shared::parse_handler_output(&stdout, handler.common.blocking) {
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

fn valid_sha256_label(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

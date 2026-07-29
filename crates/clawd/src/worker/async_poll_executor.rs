use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::{repo, AppState};

pub(super) fn execute_async_poll_dispatch_result(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    now_ts: i64,
    default_retry_after_seconds: i64,
) -> Option<Value> {
    if !claimed_async_poll_dispatch_ready(claimed) {
        return None;
    }
    let job_id = poll_job_id(claimed)?;
    let owned_adapter_result = local_process_async_poll_adapter_result(claimed, job_id, now_ts);
    let adapter_result = owned_adapter_result
        .as_ref()
        .or_else(|| async_poll_adapter_result(claimed, job_id))?;
    async_poll_dispatch_result_payload_from_adapter_result(
        claimed,
        adapter_result,
        job_id,
        now_ts,
        default_retry_after_seconds,
    )
}

pub(super) async fn execute_async_poll_dispatch_result_with_state(
    state: &AppState,
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    now_ts: i64,
    default_retry_after_seconds: i64,
) -> Option<Value> {
    if let Some(payload) =
        execute_async_poll_dispatch_result(claimed, now_ts, default_retry_after_seconds)
    {
        return Some(payload);
    }
    if !claimed_async_poll_dispatch_ready(claimed) {
        return None;
    }
    let job_id = poll_job_id(claimed)?;
    let adapter_result = skill_poll_async_adapter_result(state, claimed, job_id).await?;
    async_poll_dispatch_result_payload_from_adapter_result(
        claimed,
        &adapter_result,
        job_id,
        now_ts,
        default_retry_after_seconds,
    )
}

fn claimed_async_poll_dispatch_ready(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
) -> bool {
    claimed.task_checkpoint.checkpoint_id == claimed.checkpoint_id
        && claimed.executor_action == "poll_async_job"
        && claimed.executor_status == "async_poll_adapter_pending"
        && claimed.dispatch_state == "ready_to_poll_async_job"
        && claimed.dispatch_execution_state == "claimed_to_poll_async_job"
        && claimed.resume_directive == "poll_async_job"
        && matches!(
            claimed.task_checkpoint.resume_entrypoint,
            crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob
        )
        && claimed.execution_plan.get("text").is_none()
        && claimed.execution_plan.get("error_text").is_none()
        && claimed.dispatch_payload.get("text").is_none()
        && claimed.dispatch_payload.get("error_text").is_none()
        && claimed.dispatch_claim.get("text").is_none()
        && claimed.dispatch_claim.get("error_text").is_none()
}

fn poll_job_id(claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution) -> Option<&str> {
    claimed
        .execution_plan
        .get("job_id")
        .or_else(|| claimed.dispatch_payload.get("job_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn async_poll_adapter_result<'a>(
    claimed: &'a repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    job_id: &str,
) -> Option<&'a Value> {
    [
        &claimed.dispatch_payload,
        &claimed.execution_plan,
        &claimed.dispatch_claim,
    ]
    .into_iter()
    .filter_map(|value| value.get(crate::async_job_contract::ASYNC_POLL_ADAPTER_RESULT_KEY))
    .find(|value| crate::async_job_contract::async_poll_adapter_result_matches_job(value, job_id))
    .or_else(|| {
        claimed.task_checkpoint.observations.iter().find(|value| {
            crate::async_job_contract::async_poll_adapter_result_matches_job(value, job_id)
        })
    })
}

fn local_process_async_poll_adapter_result(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    job_id: &str,
    now_ts: i64,
) -> Option<Value> {
    if !job_id.starts_with("local_process:") {
        return None;
    }
    let job = claimed.task_checkpoint.pending_async_job.as_ref()?;
    let job_dir = job.cancel_ref.strip_prefix("local_process:")?.trim();
    if job_dir.is_empty() {
        return None;
    }
    let job_dir = std::path::Path::new(job_dir);
    let expires_at = job.expires_at;
    let exit_code_path = job_dir.join("exit_code");
    let cancel_requested_path = job_dir.join("cancel_requested_at");
    if cancel_requested_path.exists() {
        let cancel_escalation = crate::local_process_job::maybe_escalate_cancel(job_dir, now_ts);
        let pid = crate::local_process_job::read_pid(job_dir);
        let identity_state = pid
            .map(|pid| crate::local_process_job::process_identity_state(job_dir, pid))
            .unwrap_or(crate::local_process_job::ProcessIdentityState::Missing);
        let exit_code = std::fs::read_to_string(&exit_code_path)
            .ok()
            .and_then(|value| value.trim().parse::<i32>().ok());
        let stdout = crate::local_process_job::read_output_delta(
            &job_dir.join("stdout"),
            &job_dir.join("stdout_poll_cursor"),
            16 * 1024,
        );
        let stderr = crate::local_process_job::read_output_delta(
            &job_dir.join("stderr"),
            &job_dir.join("stderr_poll_cursor"),
            16 * 1024,
        );
        return Some(json!({
            "job_id": job_id,
            "adapter_kind": "local_process_poll",
            "status": "cancelled",
            "poll_after_seconds": job.poll_after_seconds,
            "expires_at": expires_at,
            "message_key": "clawd.task.cancelled",
            "cancellation_result_json": {
                "schema_version": 1,
                "source": "local_process_async_job",
                "job_id": job_id,
                "cancel_ref": job.cancel_ref,
                "terminal_reason": "cancelled",
                "pid": pid,
                "exit_code": exit_code,
                "process_identity_state": identity_state.as_token(),
                "cancel_escalation": cancel_escalation,
                "stdout": stdout.text,
                "stdout_start_cursor": stdout.start_cursor,
                "stdout_cursor": stdout.end_cursor,
                "stdout_total_bytes": stdout.total_bytes,
                "stderr": stderr.text,
                "stderr_start_cursor": stderr.start_cursor,
                "stderr_cursor": stderr.end_cursor,
                "stderr_total_bytes": stderr.total_bytes,
            }
        }));
    }
    if !exit_code_path.exists() {
        let pid = crate::local_process_job::read_pid(job_dir);
        let identity_state = pid
            .map(|pid| crate::local_process_job::process_identity_state(job_dir, pid))
            .unwrap_or(crate::local_process_job::ProcessIdentityState::Missing);
        let process_loss_stable = pid.is_none()
            || crate::local_process_job::process_loss_is_stable(job_dir, identity_state, now_ts, 5);
        let stdout = crate::local_process_job::read_output_delta(
            &job_dir.join("stdout"),
            &job_dir.join("stdout_poll_cursor"),
            16 * 1024,
        );
        let stderr = crate::local_process_job::read_output_delta(
            &job_dir.join("stderr"),
            &job_dir.join("stderr_poll_cursor"),
            16 * 1024,
        );
        let started_at = crate::local_process_job::read_i64(job_dir, "started_at");
        let runtime_timeout_seconds =
            crate::local_process_job::read_runtime_timeout_seconds(job_dir);
        let runtime_deadline_at =
            started_at
                .zip(runtime_timeout_seconds)
                .map(|(started_at, timeout_seconds)| {
                    started_at.saturating_add(timeout_seconds.min(i64::MAX as u64) as i64)
                });
        let retention_seconds = crate::local_process_job::read_retention_seconds(job_dir)
            .unwrap_or_else(|| expires_at.saturating_sub(now_ts).max(1) as u64);
        let renewed_retention_deadline_at =
            now_ts.saturating_add(retention_seconds.min(i64::MAX as u64) as i64);
        let process_observation = json!({
            "schema_version": 1,
            "source": "local_process_job_supervisor",
            "job_id": job_id,
            "pid": pid,
            "process_identity_state": identity_state.as_token(),
            "process_alive": identity_state.alive(),
            "process_loss_stable": process_loss_stable,
            "started_at": started_at,
            "updated_at": now_ts,
            "runtime_timeout_seconds": runtime_timeout_seconds,
            "runtime_deadline_at": runtime_deadline_at,
            "retention_seconds": retention_seconds,
            "retention_deadline_at": renewed_retention_deadline_at,
            "stdout": stdout.text,
            "stdout_start_cursor": stdout.start_cursor,
            "stdout_cursor": stdout.end_cursor,
            "stdout_total_bytes": stdout.total_bytes,
            "stdout_truncated": stdout.truncated,
            "stdout_cursor_reset": stdout.cursor_reset,
            "stdout_encoding": stdout.encoding,
            "stderr": stderr.text,
            "stderr_start_cursor": stderr.start_cursor,
            "stderr_cursor": stderr.end_cursor,
            "stderr_total_bytes": stderr.total_bytes,
            "stderr_truncated": stderr.truncated,
            "stderr_cursor_reset": stderr.cursor_reset,
            "stderr_encoding": stderr.encoding,
        });
        if process_loss_stable {
            return Some(json!({
                "job_id": job_id,
                "adapter_kind": "local_process_poll",
                "status": "failed",
                "poll_after_seconds": job.poll_after_seconds,
                "expires_at": renewed_retention_deadline_at,
                "error_code": match (pid, identity_state) {
                    (None, _) => "local_process_pid_missing",
                    (Some(_), crate::local_process_job::ProcessIdentityState::IdentityMismatch) => {
                        "local_process_identity_mismatch"
                    }
                    _ => "local_process_process_missing",
                },
                "message_key": "clawd.task.async_job_process_lost",
                "retryable": false,
                "failure_result_json": process_observation,
            }));
        }
        return Some(json!({
            "job_id": job_id,
            "adapter_kind": "local_process_poll",
            "status": "running",
            "poll_after_seconds": job.poll_after_seconds,
            "expires_at": renewed_retention_deadline_at,
            "runtime_deadline_at": runtime_deadline_at,
            "retention_deadline_at": renewed_retention_deadline_at,
            "message_key": job.message_key,
            "retryable": true,
            "process_observation": process_observation,
        }));
    }
    let exit_code_text = match std::fs::read_to_string(&exit_code_path) {
        Ok(value) => value,
        Err(_) => {
            return Some(local_process_exit_record_failure(
                job,
                job_id,
                "local_process_exit_record_unreadable",
            ));
        }
    };
    let exit_code = match exit_code_text.trim().parse::<i32>() {
        Ok(value) => value,
        Err(_) => {
            return Some(local_process_exit_record_failure(
                job,
                job_id,
                "local_process_exit_record_invalid",
            ));
        }
    };
    let stdout = read_bounded_output(&job_dir.join("stdout"), 32 * 1024);
    let stderr = read_bounded_output(&job_dir.join("stderr"), 32 * 1024);
    let result_json =
        local_process_terminal_result_json(claimed, job_id, job_dir, exit_code, &stdout, &stderr);
    let terminal_reason = local_process_terminal_reason(job_dir, exit_code);
    if exit_code == 0 {
        Some(json!({
            "job_id": job_id,
            "adapter_kind": "local_process_poll",
            "status": "succeeded",
            "poll_after_seconds": job.poll_after_seconds,
            "expires_at": expires_at,
            "runtime_deadline_at": job.runtime_deadline_at,
            "retention_deadline_at": job.retention_deadline_at.unwrap_or(expires_at),
            "message_key": job.message_key,
            "final_result_json": result_json
        }))
    } else {
        Some(json!({
            "job_id": job_id,
            "adapter_kind": "local_process_poll",
            "status": "failed",
            "poll_after_seconds": job.poll_after_seconds,
            "expires_at": expires_at,
            "runtime_deadline_at": job.runtime_deadline_at,
            "retention_deadline_at": job.retention_deadline_at.unwrap_or(expires_at),
            "error_code": match terminal_reason {
                "runtime_timeout" => "local_process_runtime_timeout",
                "timeout_backend_unavailable" => "local_process_timeout_backend_unavailable",
                _ => "local_process_nonzero_exit",
            },
            "message_key": match terminal_reason {
                "runtime_timeout" => "clawd.task.async_job_runtime_timeout",
                "timeout_backend_unavailable" => "clawd.task.async_job_timeout_backend_unavailable",
                _ => "clawd.task.async_job_failed",
            },
            "retryable": false,
            "failure_result_json": result_json
        }))
    }
}

fn local_process_exit_record_failure(
    job: &crate::task_lifecycle::AsyncJobRef,
    job_id: &str,
    error_code: &str,
) -> Value {
    json!({
        "job_id": job_id,
        "adapter_kind": "local_process_poll",
        "status": "failed",
        "poll_after_seconds": job.poll_after_seconds,
        "expires_at": job.expires_at,
        "error_code": error_code,
        "message_key": "clawd.task.async_job_exit_record_invalid",
        "retryable": false,
        "failure_result_json": {
            "schema_version": 1,
            "source": "local_process_job_supervisor",
            "job_id": job_id,
            "error_code": error_code,
        },
    })
}

struct BoundedProcessOutput {
    text: String,
    total_bytes: u64,
    preview_bytes: usize,
    truncated: bool,
    encoding: &'static str,
}

fn read_bounded_output(path: &Path, max_bytes: usize) -> BoundedProcessOutput {
    let total_bytes = std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let mut bytes = Vec::with_capacity(max_bytes.min(total_bytes as usize));
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(max_bytes as u64).read_to_end(&mut bytes);
    }
    let encoding = if std::str::from_utf8(&bytes).is_ok() {
        "utf-8"
    } else {
        "utf-8-lossy"
    };
    BoundedProcessOutput {
        text: String::from_utf8_lossy(&bytes).to_string(),
        total_bytes,
        preview_bytes: bytes.len(),
        truncated: total_bytes > bytes.len() as u64,
        encoding,
    }
}

fn local_process_terminal_result_json(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    job_id: &str,
    job_dir: &Path,
    exit_code: i32,
    stdout: &BoundedProcessOutput,
    stderr: &BoundedProcessOutput,
) -> Value {
    let output = combine_local_process_output(&stdout.text, &stderr.text);
    let (artifact_refs, range_handles, artifact_publish_status) =
        publish_local_process_artifacts(claimed, job_id, job_dir);
    json!({
        "schema_version": 1,
        "source": "local_process_async_job",
        "job_id": job_id,
        "pid": crate::local_process_job::read_pid(job_dir),
        "exit_code": exit_code,
        "terminal_reason": local_process_terminal_reason(job_dir, exit_code),
        "started_at": crate::local_process_job::read_i64(job_dir, "started_at"),
        "finished_at": crate::local_process_job::read_i64(job_dir, "finished_at"),
        "runtime_timeout_seconds": crate::local_process_job::read_runtime_timeout_seconds(job_dir),
        "stdout": stdout.text,
        "stderr": stderr.text,
        "output": output,
        "stdout_total_bytes": stdout.total_bytes,
        "stdout_start_cursor": 0,
        "stdout_cursor": stdout.preview_bytes,
        "stderr_total_bytes": stderr.total_bytes,
        "stderr_start_cursor": 0,
        "stderr_cursor": stderr.preview_bytes,
        "stdout_preview_bytes": stdout.preview_bytes,
        "stderr_preview_bytes": stderr.preview_bytes,
        "stdout_encoding": stdout.encoding,
        "stderr_encoding": stderr.encoding,
        "output_truncated": stdout.truncated || stderr.truncated,
        "truncated": stdout.truncated || stderr.truncated,
        "artifact_refs": artifact_refs,
        "artifacts": artifact_refs,
        "range_handles": range_handles,
        "artifact_publish_status": artifact_publish_status,
    })
}

fn local_process_terminal_reason(job_dir: &Path, exit_code: i32) -> &'static str {
    if job_dir.join("cancel_requested_at").exists() {
        "cancelled"
    } else if exit_code == 125
        && std::fs::read_to_string(job_dir.join("stderr"))
            .ok()
            .is_some_and(|stderr| stderr.contains("portable_timeout_backend_unavailable"))
    {
        "timeout_backend_unavailable"
    } else if matches!(exit_code, 124 | 125)
        && crate::local_process_job::read_runtime_timeout_seconds(job_dir).is_some()
    {
        "runtime_timeout"
    } else if exit_code == 0 {
        "exited_success"
    } else {
        "exited_nonzero"
    }
}

fn publish_local_process_artifacts(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    job_id: &str,
    job_dir: &Path,
) -> (Vec<Value>, Vec<Value>, &'static str) {
    let Some(workspace_root) = workspace_root_from_local_job_dir(job_dir) else {
        return (Vec::new(), Vec::new(), "workspace_root_unavailable");
    };
    let mut artifact_refs = Vec::new();
    let mut range_handles = Vec::new();
    let mut failed = false;
    for stream in ["stdout", "stderr"] {
        match crate::skill_output_artifact::publish_existing_task_artifact(
            &workspace_root,
            &claimed.task_id,
            "async-process",
            &job_dir.join(stream),
            &format!("{stream}.log"),
            "text/plain; charset=utf-8",
            json!({
                "stream": stream,
                "job_id": job_id,
                "adapter_kind": "local_process_poll",
            }),
        ) {
            Ok(Some(published)) => {
                artifact_refs.push(published.artifact_ref);
                range_handles.push(published.range_handle);
            }
            Ok(None) => {}
            Err(_) => failed = true,
        }
    }
    let status = if failed {
        "partial"
    } else if artifact_refs.is_empty() {
        "empty"
    } else {
        "published"
    };
    (artifact_refs, range_handles, status)
}

fn workspace_root_from_local_job_dir(job_dir: &Path) -> Option<PathBuf> {
    let async_jobs = job_dir.parent()?;
    if async_jobs.file_name()?.to_str()? != "async_jobs" {
        return None;
    }
    let rustclaw_dir = async_jobs.parent()?;
    if rustclaw_dir.file_name()?.to_str()? != ".rustclaw" {
        return None;
    }
    rustclaw_dir.parent().map(Path::to_path_buf)
}

fn combine_local_process_output(stdout: &str, stderr: &str) -> String {
    match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => format!("stdout:\n{}\n\nstderr:\n{}", stdout.trim(), stderr.trim()),
        (true, true) => String::new(),
    }
}

async fn skill_poll_async_adapter_result(
    state: &AppState,
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    job_id: &str,
) -> Option<Value> {
    let adapter = claimed
        .task_checkpoint
        .boundary_context
        .get("async_poll_adapter")
        .filter(|value| value.is_object())?;
    if adapter.get("text").is_some() || adapter.get("error_text").is_some() {
        return Some(skill_poll_failed_adapter_result(
            job_id,
            "skill_poll_adapter_text_fields_forbidden",
            "clawd.task.async_poll_adapter_failed",
            None,
        ));
    }
    let adapter_kind = adapter
        .get("adapter_kind")
        .or_else(|| adapter.get("kind"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if !adapter_kind
        .is_some_and(crate::async_job_contract::skill_runner_poll_adapter_kind_supported)
    {
        return Some(skill_poll_failed_adapter_result(
            job_id,
            "skill_poll_adapter_kind_unsupported",
            "clawd.task.async_poll_adapter_failed",
            None,
        ));
    }
    let Some(skill_name) = adapter
        .get("skill_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Some(skill_poll_failed_adapter_result(
            job_id,
            "skill_poll_adapter_missing_skill_name",
            "clawd.task.async_poll_adapter_failed",
            None,
        ));
    };
    let mut args = adapter
        .get("args")
        .cloned()
        .unwrap_or_else(|| json!({"action": "poll"}));
    let Some(obj) = args.as_object_mut() else {
        return Some(skill_poll_failed_adapter_result(
            job_id,
            "skill_poll_adapter_args_invalid",
            "clawd.task.async_poll_adapter_failed",
            None,
        ));
    };
    if obj.get("text").is_some() || obj.get("error_text").is_some() {
        return Some(skill_poll_failed_adapter_result(
            job_id,
            "skill_poll_adapter_args_text_fields_forbidden",
            "clawd.task.async_poll_adapter_failed",
            None,
        ));
    }
    obj.entry("action".to_string()).or_insert(json!("poll"));
    obj.entry("job_id".to_string()).or_insert(json!(job_id));

    match crate::run_skill_with_runner_outcome(state, &claimed.task, skill_name, args).await {
        Ok(outcome) => {
            let Some(extra) = outcome.extra else {
                return Some(skill_poll_failed_adapter_result(
                    job_id,
                    "skill_poll_adapter_result_missing",
                    "clawd.task.async_poll_adapter_failed",
                    Some(json!({
                        "source": "skill_poll_adapter",
                        "skill_name": skill_name,
                        "error_code": "missing_extra",
                    })),
                ));
            };
            if let Some(result) = skill_poll_adapter_result_from_extra(&extra, job_id) {
                return Some(result);
            }
            Some(skill_poll_failed_adapter_result(
                job_id,
                "skill_poll_adapter_result_invalid",
                "clawd.task.async_poll_adapter_failed",
                Some(json!({
                    "source": "skill_poll_adapter",
                    "skill_name": skill_name,
                    "error_code": "invalid_adapter_result",
                })),
            ))
        }
        Err(_) => Some(skill_poll_failed_adapter_result(
            job_id,
            "skill_poll_adapter_execution_failed",
            "clawd.task.async_poll_adapter_failed",
            Some(json!({
                "source": "skill_poll_adapter",
                "skill_name": skill_name,
                "error_code": "execution_failed",
            })),
        )),
    }
}

fn skill_poll_adapter_result_from_extra(extra: &Value, job_id: &str) -> Option<Value> {
    extra
        .get(crate::async_job_contract::ASYNC_POLL_ADAPTER_RESULT_KEY)
        .or(Some(extra))
        .filter(|value| {
            crate::async_job_contract::async_poll_adapter_result_matches_job(value, job_id)
        })
        .cloned()
}

fn skill_poll_failed_adapter_result(
    job_id: &str,
    error_code: &str,
    message_key: &str,
    failure_result_json: Option<Value>,
) -> Value {
    let mut result = json!({
        "job_id": job_id,
        "status": "failed",
        "error_code": error_code,
        "message_key": message_key,
    });
    if let (Some(obj), Some(failure)) = (
        result.as_object_mut(),
        failure_result_json.filter(Value::is_object),
    ) {
        obj.insert("failure_result_json".to_string(), failure);
    }
    result
}

fn async_poll_dispatch_result_payload_from_adapter_result(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    adapter_result: &Value,
    job_id: &str,
    now_ts: i64,
    default_retry_after_seconds: i64,
) -> Option<Value> {
    let adapter_status = crate::async_job_contract::async_poll_adapter_status(adapter_result)?;
    let mut payload =
        base_async_poll_result_payload(claimed, adapter_result, job_id, adapter_status);
    match adapter_status {
        "accepted" | "running" => {
            let retry_after_seconds =
                poll_retry_after_seconds(claimed, adapter_result, default_retry_after_seconds);
            let supplied_expires_at = poll_expires_at(claimed, adapter_result).unwrap_or(0);
            // A successful `running` observation proves that the adapter job is
            // alive. A stale poll/retention window must be renewed, not rewritten
            // as an underlying runtime failure. Only explicit adapter status
            // `expired` is terminal.
            let minimum_retention_seconds = adapter_result
                .get("retention_seconds")
                .and_then(Value::as_i64)
                .filter(|seconds| *seconds > 0)
                .unwrap_or_else(|| retry_after_seconds.saturating_mul(4).max(60));
            let renewed_expires_at = now_ts.saturating_add(minimum_retention_seconds);
            let expires_at = supplied_expires_at
                .max(renewed_expires_at)
                .max(now_ts.saturating_add(retry_after_seconds).saturating_add(1));
            let retention_renewed = expires_at != supplied_expires_at;
            let next_check_after = now_ts.saturating_add(retry_after_seconds).min(expires_at);
            let obj = payload.as_object_mut()?;
            obj.insert(
                "executor_result_status".to_string(),
                json!("async_poll_rescheduled"),
            );
            obj.insert(
                "reason_code".to_string(),
                json!(match adapter_status {
                    "accepted" => "async_poll_accepted",
                    _ => "async_poll_running",
                }),
            );
            obj.insert(
                "defer_reason_code".to_string(),
                json!(match adapter_status {
                    "accepted" => "async_poll_accepted",
                    _ => "async_poll_running",
                }),
            );
            obj.insert(
                "retry_after_seconds".to_string(),
                json!(retry_after_seconds),
            );
            obj.insert("next_check_after".to_string(), json!(next_check_after));
            obj.insert("expires_at".to_string(), json!(expires_at));
            obj.insert("retention_deadline_at".to_string(), json!(expires_at));
            obj.insert("retention_renewed".to_string(), json!(retention_renewed));
            Some(payload)
        }
        "succeeded" => {
            let final_result_json = adapter_result
                .get("final_result_json")
                .cloned()
                .filter(Value::is_object)?;
            let continuation_result_json =
                crate::agent_engine::completed_async_job_continuation_result(
                    &claimed.task.kind,
                    &claimed.task_checkpoint,
                    &final_result_json,
                    now_ts,
                );
            let obj = payload.as_object_mut()?;
            obj.insert(
                "executor_result_status".to_string(),
                json!("async_poll_completed"),
            );
            obj.insert("reason_code".to_string(), json!("async_poll_completed"));
            obj.insert("final_result_json".to_string(), final_result_json);
            if let Some(continuation_result_json) = continuation_result_json {
                obj.insert(
                    "continuation_result_json".to_string(),
                    continuation_result_json,
                );
            }
            Some(payload)
        }
        "failed" => {
            let (error_code, message_key) = adapter_error_fields(
                adapter_result,
                "async_poll_adapter_failed",
                "clawd.task.async_poll_adapter_failed",
            );
            async_poll_failure_payload(
                payload,
                "async_poll_failed",
                error_code,
                message_key,
                adapter_result.get("failure_result_json").cloned(),
            )
        }
        "expired" => async_poll_failure_payload(
            payload,
            "async_poll_failed",
            "async_poll_expired",
            "clawd.task.async_poll_expired",
            adapter_result.get("failure_result_json").cloned(),
        ),
        "cancelled" => {
            let obj = payload.as_object_mut()?;
            obj.insert(
                "executor_result_status".to_string(),
                json!("async_poll_cancelled"),
            );
            obj.insert("reason_code".to_string(), json!("async_poll_cancelled"));
            obj.insert("message_key".to_string(), json!("clawd.task.cancelled"));
            if let Some(cancellation_result_json) = adapter_result
                .get("cancellation_result_json")
                .cloned()
                .filter(Value::is_object)
            {
                obj.insert(
                    "cancellation_result_json".to_string(),
                    cancellation_result_json,
                );
            }
            Some(payload)
        }
        _ => None,
    }
}

fn base_async_poll_result_payload(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    adapter_result: &Value,
    job_id: &str,
    adapter_status: &str,
) -> Value {
    let mut payload = json!({
        "schema_version": 1,
        "task_id": claimed.task_id,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_state": claimed.executor_state,
        "executor_action": claimed.executor_action,
        "executor_status": claimed.executor_status,
        "dispatch_state": claimed.dispatch_state,
        "dispatch_execution_state": claimed.dispatch_execution_state,
        "resume_trigger": claimed.resume_trigger,
        "resume_directive": claimed.resume_directive,
        "lease_expires_at": claimed.lease_expires_at,
        "handoff_claim_expires_at": claimed.handoff_claim_expires_at,
        "dispatch_claim_expires_at": claimed.dispatch_claim_expires_at,
        "completed_side_effect_count": claimed.task_checkpoint.completed_side_effect_refs.len(),
        "job_id": job_id,
        "adapter_status": adapter_status,
    });
    if let Some(obj) = payload.as_object_mut() {
        for key in [
            "process_observation",
            "retryable",
            "runtime_deadline_at",
            "retention_deadline_at",
        ] {
            if let Some(value) = adapter_result.get(key) {
                obj.insert(key.to_string(), value.clone());
            }
        }
        for key in ["cancel_ref", "message_key"] {
            if let Some(value) = claimed
                .execution_plan
                .get(key)
                .or_else(|| claimed.dispatch_payload.get(key))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                obj.insert(key.to_string(), json!(value));
            }
        }
        if let Some(adapter_kind) = adapter_result
            .get("adapter_kind")
            .or_else(|| adapter_result.get("kind"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| crate::async_job_contract::async_poll_adapter_kind_supported(value))
        {
            obj.insert("adapter_kind".to_string(), json!(adapter_kind));
        }
    }
    payload
}

fn poll_expires_at(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    adapter_result: &Value,
) -> Option<i64> {
    adapter_result
        .get("expires_at")
        .or_else(|| claimed.execution_plan.get("expires_at"))
        .or_else(|| claimed.dispatch_payload.get("expires_at"))
        .and_then(Value::as_i64)
}

fn poll_retry_after_seconds(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    adapter_result: &Value,
    default_retry_after_seconds: i64,
) -> i64 {
    adapter_result
        .get("poll_after_seconds")
        .or_else(|| claimed.execution_plan.get("poll_after_seconds"))
        .or_else(|| claimed.dispatch_payload.get("poll_after_seconds"))
        .and_then(Value::as_i64)
        .filter(|seconds| *seconds > 0)
        .unwrap_or(default_retry_after_seconds.max(1))
}

fn adapter_error_fields<'a>(
    adapter_result: &'a Value,
    default_error_code: &'static str,
    default_message_key: &'static str,
) -> (&'a str, &'a str) {
    let error_code = adapter_result
        .get("error_code")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_error_code);
    let message_key = adapter_result
        .get("message_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_message_key);
    (error_code, message_key)
}

fn async_poll_failure_payload(
    mut payload: Value,
    executor_result_status: &str,
    error_code: &str,
    message_key: &str,
    failure_result_json: Option<Value>,
) -> Option<Value> {
    let obj = payload.as_object_mut()?;
    obj.insert(
        "executor_result_status".to_string(),
        json!(executor_result_status),
    );
    obj.insert("reason_code".to_string(), json!(error_code));
    obj.insert("error_code".to_string(), json!(error_code));
    obj.insert("message_key".to_string(), json!(message_key));
    if let Some(failure_result_json) = failure_result_json.filter(Value::is_object) {
        obj.insert("failure_result_json".to_string(), failure_result_json);
    }
    Some(payload)
}

#[cfg(test)]
#[path = "async_poll_executor_tests.rs"]
mod tests;

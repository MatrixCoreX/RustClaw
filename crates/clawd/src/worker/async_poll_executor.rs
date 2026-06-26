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
    if !exit_code_path.exists() && cancel_requested_path.exists() {
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
            }
        }));
    }
    if !exit_code_path.exists() {
        return Some(json!({
            "job_id": job_id,
            "adapter_kind": "local_process_poll",
            "status": if now_ts >= expires_at { "expired" } else { "running" },
            "poll_after_seconds": job.poll_after_seconds,
            "expires_at": expires_at,
            "message_key": job.message_key,
        }));
    }
    let exit_code_text = std::fs::read_to_string(&exit_code_path).ok()?;
    let exit_code = exit_code_text.trim().parse::<i32>().ok()?;
    let stdout = read_bounded_utf8(&job_dir.join("stdout"), 32 * 1024);
    let stderr = read_bounded_utf8(&job_dir.join("stderr"), 32 * 1024);
    let output = combine_local_process_output(&stdout, &stderr);
    if exit_code == 0 {
        Some(json!({
            "job_id": job_id,
            "adapter_kind": "local_process_poll",
            "status": "succeeded",
            "poll_after_seconds": job.poll_after_seconds,
            "expires_at": expires_at,
            "message_key": job.message_key,
            "final_result_json": {
                "schema_version": 1,
                "source": "local_process_async_job",
                "job_id": job_id,
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "output": output,
            }
        }))
    } else {
        Some(json!({
            "job_id": job_id,
            "adapter_kind": "local_process_poll",
            "status": "failed",
            "poll_after_seconds": job.poll_after_seconds,
            "expires_at": expires_at,
            "error_code": "local_process_nonzero_exit",
            "message_key": "clawd.task.async_job_failed",
            "failure_result_json": {
                "schema_version": 1,
                "source": "local_process_async_job",
                "job_id": job_id,
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "output": output,
            }
        }))
    }
}

fn read_bounded_utf8(path: &std::path::Path, max_bytes: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let limit = bytes.len().min(max_bytes);
    let mut text = String::from_utf8_lossy(&bytes[..limit]).to_string();
    if bytes.len() > limit {
        text.push_str("...");
    }
    text
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
                        "error_kind": "missing_extra",
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
                    "error_kind": "invalid_adapter_result",
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
                "error_kind": "execution_failed",
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
            let expires_at = poll_expires_at(claimed, adapter_result)?;
            if expires_at <= now_ts {
                return async_poll_failure_payload(
                    payload,
                    "async_poll_failed",
                    "async_poll_expired",
                    "clawd.task.async_poll_expired",
                    None,
                );
            }
            let retry_after_seconds =
                poll_retry_after_seconds(claimed, adapter_result, default_retry_after_seconds);
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
            Some(payload)
        }
        "succeeded" => {
            let final_result_json = adapter_result
                .get("final_result_json")
                .cloned()
                .filter(Value::is_object)?;
            let obj = payload.as_object_mut()?;
            obj.insert(
                "executor_result_status".to_string(),
                json!("async_poll_completed"),
            );
            obj.insert("reason_code".to_string(), json!("async_poll_completed"));
            obj.insert("final_result_json".to_string(), final_result_json);
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

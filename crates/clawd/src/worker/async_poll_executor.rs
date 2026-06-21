use serde_json::{json, Value};

use crate::repo;

pub(super) fn execute_async_poll_dispatch_result(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    now_ts: i64,
    default_retry_after_seconds: i64,
) -> Option<Value> {
    if !claimed_async_poll_dispatch_ready(claimed) {
        return None;
    }
    let job_id = poll_job_id(claimed)?;
    let adapter_result = async_poll_adapter_result(claimed, job_id)?;
    async_poll_dispatch_result_payload_from_adapter_result(
        claimed,
        adapter_result,
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

fn async_poll_dispatch_result_payload_from_adapter_result(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
    adapter_result: &Value,
    job_id: &str,
    now_ts: i64,
    default_retry_after_seconds: i64,
) -> Option<Value> {
    let adapter_status = crate::async_job_contract::async_poll_adapter_status(adapter_result)?;
    let mut payload = base_async_poll_result_payload(claimed, job_id, adapter_status);
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
        _ => None,
    }
}

fn base_async_poll_result_payload(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
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

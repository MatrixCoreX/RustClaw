use serde_json::{json, Value};

pub(super) fn image_pending_async_job_contract(
    provider: &str,
    model: &str,
    job_id: &str,
    task_id: &str,
    output_path: &str,
    poll_after_seconds: u64,
    expires_at: i64,
) -> Value {
    json!({
        "job_id": job_id,
        "provider": provider,
        "status": "accepted",
        "poll_after_seconds": poll_after_seconds,
        "poll_after_ms": poll_after_seconds.saturating_mul(1_000),
        "expires_at": expires_at,
        "cancel_ref": job_id,
        "cancel_token": job_id,
        "result_ref": job_id,
        "message_key": "clawd.task.async_job_pending",
        "retryable": true,
        "poll_adapter": {
            "kind": "media_job_poll",
            "skill_name": "image_generate",
            "args": {
                "action": "poll",
                "task_id": task_id,
                "job_id": job_id,
                "vendor": provider,
                "model": model,
                "output_path": output_path,
                "poll_after_seconds": poll_after_seconds,
                "expires_at": expires_at,
                "dry_run": task_id == "dry_run",
            }
        }
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn image_poll_response(
    task_id: &str,
    job_id: &str,
    provider: &str,
    model: &str,
    model_kind: &str,
    poll_after_seconds: u64,
    expires_at: i64,
    adapter_result: Value,
    query: Value,
) -> (String, Value) {
    (
        format!("IMAGE_TASK:{task_id}"),
        json!({
            "provider": provider,
            "model": model,
            "model_kind": model_kind,
            "task_id": task_id,
            "job_id": job_id,
            "status": query.get("status").cloned().unwrap_or(Value::Null),
            "poll_after_seconds": poll_after_seconds,
            "expires_at": expires_at,
            "query": query,
            "async_poll_adapter_result": adapter_result,
        }),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn image_poll_adapter_result(
    task_id: &str,
    job_id: &str,
    provider: &str,
    model: &str,
    model_kind: &str,
    poll_after_seconds: u64,
    expires_at: i64,
    raw_status: &str,
    output_path: Option<&str>,
    dry_run: bool,
    error_code: Option<&str>,
    message_key: Option<&str>,
) -> Result<Value, String> {
    let status = normalized_image_async_status(raw_status);
    let retryable = matches!(status, "accepted" | "running");
    let message_key = message_key.unwrap_or(match status {
        "succeeded" => "clawd.task.async_job_completed",
        "failed" => "clawd.task.async_poll_adapter_failed",
        "expired" => "clawd.task.async_poll_expired",
        "cancelled" => "clawd.task.cancelled",
        _ => "clawd.task.async_job_pending",
    });
    let mut result = json!({
        "schema_version": 1,
        "adapter_kind": "media_job_poll",
        "status": status,
        "job_id": job_id,
        "result_ref": job_id,
        "poll_after_seconds": poll_after_seconds,
        "poll_after_ms": poll_after_seconds.saturating_mul(1_000),
        "expires_at": expires_at,
        "message_key": message_key,
        "retryable": retryable,
    });
    let Some(map) = result.as_object_mut() else {
        return Err("adapter_result must be object".to_string());
    };
    match status {
        "succeeded" => {
            let output = output_path
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("image/gen-{task_id}.png"));
            map.insert(
                "final_result_json".to_string(),
                json!({
                    "schema_version": 1,
                    "source": "image_generate_poll_adapter",
                    "provider": provider,
                    "model": model,
                    "model_kind": model_kind,
                    "task_id": task_id,
                    "job_id": job_id,
                    "output_path": output,
                    "outputs": [{"type": "image_file", "path": output}],
                    "dry_run": dry_run,
                }),
            );
        }
        "failed" | "expired" => {
            map.insert(
                "error_code".to_string(),
                Value::String(
                    error_code
                        .unwrap_or(if status == "expired" {
                            "async_poll_expired"
                        } else {
                            "provider_image_job_failed"
                        })
                        .to_string(),
                ),
            );
            map.insert(
                "failure_result_json".to_string(),
                json!({
                    "schema_version": 1,
                    "source": "image_generate_poll_adapter",
                    "provider": provider,
                    "model": model,
                    "model_kind": model_kind,
                    "task_id": task_id,
                    "job_id": job_id,
                    "status": status,
                    "dry_run": dry_run,
                }),
            );
        }
        "cancelled" => {
            map.insert("cancel_ref".to_string(), Value::String(job_id.to_string()));
            map.insert(
                "cancel_token".to_string(),
                Value::String(job_id.to_string()),
            );
            map.insert(
                "cancellation_result_json".to_string(),
                json!({
                    "schema_version": 1,
                    "source": "image_generate_cancel_adapter",
                    "provider": provider,
                    "model": model,
                    "model_kind": model_kind,
                    "task_id": task_id,
                    "job_id": job_id,
                    "cancel_ref": job_id,
                    "status": "cancelled",
                    "dry_run": dry_run,
                }),
            );
        }
        _ => {}
    }
    Ok(result)
}

pub(super) fn image_cancelled_adapter_result(
    task_id: &str,
    job_id: &str,
    provider: &str,
    model: &str,
    model_kind: &str,
    cancelled_at: i64,
) -> Value {
    json!({
        "schema_version": 1,
        "adapter_kind": "media_job_poll",
        "status": "cancelled",
        "job_id": job_id,
        "result_ref": job_id,
        "cancel_ref": job_id,
        "cancel_token": job_id,
        "poll_after_seconds": 0,
        "poll_after_ms": 0,
        "expires_at": cancelled_at,
        "message_key": "clawd.task.cancelled",
        "retryable": false,
        "cancellation_result_json": {
            "schema_version": 1,
            "source": "image_generate_cancel_adapter",
            "provider": provider,
            "model": model,
            "model_kind": model_kind,
            "task_id": task_id,
            "job_id": job_id,
            "cancel_ref": job_id,
            "status": "cancelled",
            "dry_run": true,
        }
    })
}

fn normalized_image_async_status(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "accepted" | "queued" | "queueing" | "created" | "submitted" => "accepted",
        "running" | "processing" | "pending" | "in_progress" => "running",
        "succeeded" | "success" | "done" | "completed" | "complete" => "succeeded",
        "failed" | "fail" | "error" => "failed",
        "expired" | "timeout" | "timed_out" => "expired",
        "cancelled" | "canceled" => "cancelled",
        _ => "running",
    }
}

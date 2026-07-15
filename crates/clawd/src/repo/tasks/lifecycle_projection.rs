use serde_json::{json, Value};

use super::ReadyPausedCheckpointResumeExecutor;
use crate::truncate_for_log;

pub(super) fn append_task_lease_lifecycle_fields(
    lifecycle: &mut Value,
    lease_owner: Option<&str>,
    lease_expires_at: i64,
    claim_attempt: i64,
    claimed_at: i64,
) {
    let Some(obj) = lifecycle.as_object_mut() else {
        return;
    };
    if let Some(owner) = lease_owner.map(str::trim).filter(|value| !value.is_empty()) {
        obj.insert("lease_owner".to_string(), serde_json::json!(owner));
    }
    if lease_expires_at > 0 {
        obj.insert(
            "lease_expires_at".to_string(),
            serde_json::json!(lease_expires_at),
        );
    }
    if claim_attempt > 0 {
        obj.insert(
            "claim_attempt".to_string(),
            serde_json::json!(claim_attempt),
        );
        obj.entry("attempt_id".to_string())
            .or_insert(serde_json::json!(claim_attempt));
    }
    if claimed_at > 0 {
        obj.insert("claimed_at".to_string(), serde_json::json!(claimed_at));
    }
}

pub(super) fn append_checkpoint_resume_directive_lifecycle_fields(
    lifecycle: &mut Value,
    result_json: Option<&Value>,
) {
    let Some(result_json) = result_json else {
        return;
    };
    let Some(obj) = lifecycle.as_object_mut() else {
        return;
    };
    let state = obj
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(state, "waiting" | "background" | "needs_user") {
        return;
    }
    let directive =
        crate::task_lifecycle::checkpoint_resume_directive(result_json, crate::now_ts_u64() as i64);
    if directive.status_code() == "not_paused" {
        return;
    }
    obj.entry("resume_directive".to_string())
        .or_insert_with(|| serde_json::json!(directive.status_code()));
    obj.entry("resume_directive_payload".to_string())
        .or_insert_with(|| directive.to_machine_json());
}

pub(super) fn expired_resume_claim_recovery_metadata(
    lifecycle: &Value,
    checkpoint_id: &str,
    now_ts: i64,
) -> Option<(Option<String>, i64)> {
    let claim = lifecycle.get("resume_claim")?;
    let claim_checkpoint_id = claim
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id {
        return None;
    }
    let expires_at = claim.get("expires_at").and_then(Value::as_i64)?;
    if expires_at <= 0 || expires_at > now_ts {
        return None;
    }
    let owner = claim
        .get("owner")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Some((owner, expires_at))
}

pub(super) fn async_poll_terminal_projection_without_visible_reply(raw_result_json: &str) -> bool {
    let Ok(result_json) = serde_json::from_str::<Value>(raw_result_json) else {
        return false;
    };
    if result_has_visible_reply(&result_json) {
        return false;
    }
    let Some(lifecycle) = result_json.get("task_lifecycle") else {
        return false;
    };
    lifecycle
        .get("terminal_executor_action")
        .and_then(Value::as_str)
        == Some("poll_async_job")
        && lifecycle
            .get("terminal_executor_result_status")
            .and_then(Value::as_str)
            == Some("async_poll_completed")
        && lifecycle
            .get("resume_executor_result_projection")
            .and_then(|value| value.get("final_result_json"))
            .is_some()
}

fn result_has_visible_reply(result_json: &Value) -> bool {
    result_json
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || result_json
            .get("messages")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.as_str()
                        .map(str::trim)
                        .is_some_and(|value| !value.is_empty())
                })
            })
}

pub(super) fn worker_timeout_preserves_recoverable_checkpoint(result_json: Option<&str>) -> bool {
    let Some(raw) = result_json else {
        return false;
    };
    let Ok(result_json) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    matches!(
        crate::task_lifecycle::paused_checkpoint_recovery_status(
            &result_json,
            crate::now_ts_u64() as i64
        ),
        crate::task_lifecycle::PausedCheckpointRecoveryStatus::Waiting { .. }
    )
}

pub(super) fn worker_timeout_result_json(task_id: &str) -> String {
    let reason_code =
        crate::task_lifecycle::TerminalFailureReason::ToolTimeoutWithoutAsyncResume.status_code();
    json!({
        "schema_version": 1,
        "status_code": "worker_task_timeout",
        "reason_code": reason_code,
        "message_key": "clawd.task.worker_timeout",
        "task_lifecycle": {
            "schema_version": 1,
            "state": "failed",
            "source": "worker_timeout",
            "terminal_reason": reason_code,
            "reason_code": reason_code,
            "worker_events": [
                {
                    "event_type": "tool_timeout",
                    "owner_layer": "worker_runtime",
                    "task_id": task_id,
                    "state_from": "running",
                    "state_to": "timeout",
                    "reason_code": reason_code,
                }
            ]
        }
    })
    .to_string()
}

pub(super) fn worker_failure_result_json(task_id: &str, error_text: &str) -> String {
    let reason_code = worker_failure_reason_code(error_text);
    let failure_attribution = worker_failure_attribution(reason_code);
    let message_key = worker_failure_message_key(reason_code);
    json!({
        "schema_version": 1,
        "status_code": "worker_task_failed",
        "reason_code": reason_code,
        "message_key": message_key,
        "failure_attribution": failure_attribution,
        "task_lifecycle": {
            "schema_version": 1,
            "state": "failed",
            "source": "worker_failure",
            "terminal_reason": reason_code,
            "reason_code": reason_code,
            "failure_attribution": failure_attribution,
            "worker_events": [
                {
                    "event_type": "worker_failure",
                    "owner_layer": "worker_runtime",
                    "task_id": task_id,
                    "state_from": "running",
                    "state_to": "failed",
                    "reason_code": reason_code,
                }
            ]
        }
    })
    .to_string()
}

fn worker_failure_reason_code(error_text: &str) -> &'static str {
    let Some(structured) = crate::skills::parse_structured_skill_error(error_text.trim()) else {
        return "worker_runtime_error";
    };
    let error_kind = structured.error_kind.trim().to_ascii_lowercase();
    if matches!(error_kind.as_str(), "timeout" | "idle_timeout") {
        return crate::task_lifecycle::TerminalFailureReason::ToolTimeoutWithoutAsyncResume
            .status_code();
    }
    if error_kind == "confirmation_timeout" {
        return crate::task_lifecycle::TerminalFailureReason::ConfirmationTimeout.status_code();
    }
    if worker_failure_kind_is_provider_window(&error_kind) {
        return crate::task_lifecycle::TerminalFailureReason::ProviderWindowExhausted.status_code();
    }
    "worker_runtime_error"
}

fn worker_failure_kind_is_provider_window(error_kind: &str) -> bool {
    matches!(
        error_kind,
        "provider_error"
            | "provider_retryable_response"
            | "provider_retryable_business"
            | "provider_non_retryable_business"
            | "provider_response_invalid"
            | "provider_schema_error"
            | "provider_unavailable"
            | "provider_rate_limited"
            | "llm_provider_error"
            | "llm_provider_unavailable"
    )
}

fn worker_failure_attribution(reason_code: &str) -> &'static str {
    match reason_code {
        "provider_window_exhausted" => "provider_error",
        "confirmation_timeout" => "confirmation_wait",
        "tool_timeout_without_async_resume" => "tool_timeout",
        _ => "runtime_error",
    }
}

fn worker_failure_message_key(reason_code: &str) -> &'static str {
    match reason_code {
        "provider_window_exhausted" => "clawd.task.provider_window_exhausted",
        "confirmation_timeout" => "clawd.task.confirmation_timeout",
        "tool_timeout_without_async_resume" => "clawd.task.worker_timeout",
        _ => "clawd.task.worker_failed",
    }
}

pub(super) fn normalized_optional_task_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

pub(super) fn summarize_active_task_payload(kind: &str, payload_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(payload_json) else {
        return truncate_for_log(payload_json);
    };
    let summary = match kind {
        "ask" => v
            .get("text")
            .and_then(|x| x.as_str())
            .unwrap_or(payload_json)
            .to_string(),
        "run_skill" => {
            let skill = v
                .get("skill_name")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown");
            let action = v
                .get("args")
                .and_then(|x| x.get("action"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .trim();
            if action.is_empty() {
                format!("run_skill:{skill}")
            } else {
                format!("run_skill:{skill} action={action}")
            }
        }
        _ => payload_json.to_string(),
    };
    truncate_for_log(summary.trim())
}

pub(super) fn ready_paused_checkpoint_resume_executor_from_result_json(
    task_id: String,
    result_json: &Value,
    now_ts: i64,
) -> Option<ReadyPausedCheckpointResumeExecutor> {
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(result_json), None);
    let lifecycle_obj = lifecycle.as_object()?;
    let lifecycle_state = lifecycle_obj
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !matches!(lifecycle_state, "background" | "waiting" | "running") {
        return None;
    }
    let checkpoint_id = lifecycle_obj
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if checkpoint_id.is_empty() {
        return None;
    }
    let resume_executor = lifecycle_obj
        .get("resume_executor")
        .filter(|value| value.is_object())
        .cloned()?;
    let executor_checkpoint_id = resume_executor
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if executor_checkpoint_id != checkpoint_id {
        return None;
    }
    let executor_state = resume_executor
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if !resume_executor_state_is_ready(lifecycle_obj, executor_state, checkpoint_id, now_ts) {
        return None;
    }
    let next_check_after = lifecycle_obj
        .get("next_check_after")
        .and_then(Value::as_i64)
        .or_else(|| {
            resume_executor
                .get("next_check_after")
                .and_then(Value::as_i64)
        });
    if next_check_after.is_some_and(|ts| ts > now_ts) {
        return None;
    }
    let task_checkpoint = crate::task_lifecycle::task_checkpoint_from_result_json(result_json)?;
    if task_checkpoint.checkpoint_id != checkpoint_id {
        return None;
    }
    let resume_work_item = lifecycle_obj
        .get("resume_work_item")
        .filter(|value| value.is_object())
        .cloned();
    if let Some(work_item_checkpoint_id) = resume_work_item
        .as_ref()
        .and_then(|value| value.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
    {
        if work_item_checkpoint_id != checkpoint_id {
            return None;
        }
    }
    let resume_trigger = resume_executor
        .get("resume_trigger")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    let resume_directive = resume_executor
        .get("resume_directive")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if resume_directive.is_empty() {
        return None;
    }
    Some(ReadyPausedCheckpointResumeExecutor {
        task_id,
        lifecycle_state: lifecycle_state.to_string(),
        checkpoint_id: checkpoint_id.to_string(),
        executor_state: executor_state.to_string(),
        resume_trigger,
        resume_directive,
        next_check_after,
        resume_executor,
        resume_work_item,
        task_checkpoint,
    })
}

fn resume_executor_state_is_ready(
    lifecycle_obj: &serde_json::Map<String, Value>,
    executor_state: &str,
    checkpoint_id: &str,
    now_ts: i64,
) -> bool {
    if matches!(
        executor_state,
        "ready_for_planner_resume" | "ready_to_finalize" | "poll_scheduled"
    ) {
        return true;
    }
    if !matches!(
        executor_state,
        "executing_planner_resume" | "executing_finalize" | "executing_async_poll"
    ) {
        return false;
    }
    let claim = lifecycle_obj.get("resume_executor_claim");
    let claim_checkpoint_id = claim
        .and_then(|value| value.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id {
        return false;
    }
    claim
        .and_then(|value| value.get("expires_at"))
        .and_then(Value::as_i64)
        .is_some_and(|expires_at| expires_at <= now_ts)
}

pub(super) fn executing_resume_executor_state(executor_state: &str) -> Option<&'static str> {
    match executor_state {
        "ready_for_planner_resume" | "executing_planner_resume" => Some("executing_planner_resume"),
        "ready_to_finalize" | "executing_finalize" => Some("executing_finalize"),
        "poll_scheduled" | "executing_async_poll" => Some("executing_async_poll"),
        _ => None,
    }
}

pub(super) fn resume_entrypoint_token(
    entrypoint: crate::task_lifecycle::ResumeEntrypoint,
) -> &'static str {
    match entrypoint {
        crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound => "next_planner_round",
        crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob => "poll_async_job",
        crate::task_lifecycle::ResumeEntrypoint::AwaitUserInput => "await_user_input",
        crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize => "verify_and_finalize",
    }
}

use serde_json::{json, Value};

use claw_core::provider_failure_policy::ProviderFailurePolicy;

use super::{ActiveTaskItem, SkillInput};

pub(super) fn task_detail_input_status_extra(status: &str, task_id: Option<&str>) -> Value {
    json!({
        "schema_version": 1,
        "action": "get",
        "status": status,
        "message_key": format!("task_control.get.{status}"),
        "task_id": task_id,
        "db_status": Value::Null,
        "lifecycle": {
            "state": status,
            "can_poll": false,
            "can_cancel": false,
            "last_heartbeat_ts": Value::Null,
            "checkpoint_id": Value::Null,
        },
        "field_value": {
            "action": "get",
            "status": status,
            "message_key": format!("task_control.get.{status}"),
            "task_id": task_id,
            "db_status": Value::Null,
            "state": status,
            "can_poll": false,
            "can_cancel": false,
            "last_heartbeat_ts": Value::Null,
            "checkpoint_id": Value::Null,
        },
    })
}

pub(super) fn task_control_input_status_extra(
    action: &str,
    status: &str,
    task_id: Option<&str>,
) -> Value {
    json!({
        "schema_version": 1,
        "action": action,
        "status": status,
        "message_key": format!("task_control.{action}.{status}"),
        "task_id": task_id,
        "field_value": {
            "action": action,
            "status": status,
            "message_key": format!("task_control.{action}.{status}"),
            "task_id": task_id,
            "can_poll": false,
            "can_cancel": false,
        },
    })
}

pub(super) fn cancel_dry_run_extra(action: &str, task_id: Option<&str>) -> Value {
    json!({
        "schema_version": 1,
        "action": action,
        "status": "dry_run",
        "message_key": format!("task_control.{action}.dry_run"),
        "would_mutate": false,
        "task_id": task_id,
        "required_fields": ["task_id", "state", "can_cancel"],
        "precondition_fields": {
            "state": "running_or_queued",
            "can_cancel": true,
        },
        "result_projection_fields": {
            "state": "cancel_requested_or_canceled",
            "can_cancel": false,
            "can_poll": true,
            "db_status": "canceled_or_terminal",
            "last_heartbeat_ts": "optional",
            "checkpoint_id": "optional",
        },
        "field_value": {
            "action": action,
            "status": "dry_run",
            "message_key": format!("task_control.{action}.dry_run"),
            "would_mutate": false,
            "task_id": task_id,
            "state": "running_or_queued",
            "can_cancel": true,
            "can_poll": true,
        },
    })
}

pub(super) fn resume_dry_run_extra(input: &SkillInput) -> Value {
    resume_preview_contract(input, "resume")
}

pub(super) fn resume_preview_extra(input: &SkillInput) -> Value {
    resume_preview_contract(input, "preview_resume")
}

pub(super) fn provider_failure_preview_extra(policy: ProviderFailurePolicy) -> Value {
    let failure_class = policy.failure_class.as_str();
    json!({
        "schema_version": 1,
        "action": "preview_provider_failure",
        "status": "dry_run",
        "message_key": "task_control.preview_provider_failure.dry_run",
        "dry_run": true,
        "would_mutate": false,
        "failure_class": failure_class,
        "provider_retryable": policy.provider_retryable,
        "provider_blocker": policy.provider_blocker,
        "retry_policy": policy.retry_policy,
        "retry_after_seconds": policy.retry_after_seconds,
        "waiting_state": policy.waiting_state,
        "provider_message_key": policy.message_key,
        "checkpoint": {
            "required": policy.checkpoint_required,
            "recovery_action": policy.recovery_action,
            "resume_reason": policy.resume_reason,
            "resume_entrypoint": policy.resume_entrypoint,
        },
        "field_value": {
            "action": "preview_provider_failure",
            "status": "dry_run",
            "message_key": "task_control.preview_provider_failure.dry_run",
            "dry_run": true,
            "would_mutate": false,
            "failure_class": failure_class,
            "provider_retryable": policy.provider_retryable,
            "provider_blocker": policy.provider_blocker,
            "retry_policy": policy.retry_policy,
            "retry_after_seconds": policy.retry_after_seconds,
            "waiting_state": policy.waiting_state,
            "checkpoint_required": policy.checkpoint_required,
            "recovery_action": policy.recovery_action,
            "resume_reason": policy.resume_reason,
            "resume_entrypoint": policy.resume_entrypoint,
        },
    })
}

fn resume_preview_contract(input: &SkillInput, action: &str) -> Value {
    json!({
        "schema_version": 1,
        "action": action,
        "status": "dry_run",
        "message_key": format!("task_control.{action}.dry_run"),
        "would_mutate": false,
        "task_id": input.task_id.as_deref(),
        "checkpoint_id": input.checkpoint_id.as_deref(),
        "resume_reason": input.resume_reason.as_deref(),
        "user_message_present": input.user_message.as_ref().is_some_and(|value| !value.trim().is_empty()),
        "new_constraints_present": input.new_constraints.is_some(),
        "required_fields": ["task_id"],
        "optional_fields": ["checkpoint_id", "resume_reason", "user_message", "new_constraints"],
        "precondition_fields": {
            "state": "waiting_or_background_or_checkpointed",
            "checkpoint_id": "optional",
            "can_poll": true,
        },
        "resume_entrypoint": "checkpoint_declared",
        "lease": {
            "required": true,
            "scope": "resume_execution",
            "mode": "renewable",
            "seconds_source": "runtime_config",
            "heartbeat_renewal": true,
        },
        "result_projection_fields": {
            "state": "running_or_background_or_terminal",
            "db_status": "running_or_terminal",
            "resume_due": true,
            "can_poll": true,
            "can_cancel": true,
            "checkpoint_id": "optional",
        },
        "field_value": {
            "action": action,
            "status": "dry_run",
            "message_key": format!("task_control.{action}.dry_run"),
            "would_mutate": false,
            "task_id": input.task_id.as_deref(),
            "checkpoint_id": input.checkpoint_id.as_deref(),
            "resume_reason": input.resume_reason.as_deref(),
            "resume_entrypoint": "checkpoint_declared",
            "lease_required": true,
            "lease_scope": "resume_execution",
            "lease_mode": "renewable",
            "lease_seconds_source": "runtime_config",
            "lease_heartbeat_renewal": true,
            "can_poll": true,
            "can_cancel": true,
        },
    })
}

pub(super) fn pause_dry_run_extra(input: &SkillInput) -> Value {
    let pause_seconds = input.pause_seconds.unwrap_or(3600);
    json!({
        "schema_version": 1,
        "action": "pause",
        "status": "dry_run",
        "message_key": "task_control.pause.dry_run",
        "would_mutate": false,
        "task_id": input.task_id.as_deref(),
        "pause_seconds": pause_seconds,
        "required_fields": ["task_id"],
        "optional_fields": ["pause_seconds"],
        "precondition_fields": {
            "state": "waiting_or_background",
            "checkpoint_id": "required",
            "can_poll": true,
        },
        "result_projection_fields": {
            "state": "waiting_or_background",
            "db_status": "running",
            "resume_due": false,
            "resume_wait_seconds": pause_seconds,
            "can_poll": true,
            "can_cancel": true,
        },
        "field_value": {
            "action": "pause",
            "status": "dry_run",
            "message_key": "task_control.pause.dry_run",
            "would_mutate": false,
            "task_id": input.task_id.as_deref(),
            "pause_seconds": pause_seconds,
            "can_poll": true,
            "can_cancel": true,
        },
    })
}

pub(super) fn task_control_by_id_result_extra(
    action: &str,
    task_id: &str,
    response: Value,
) -> Value {
    let lifecycle = response
        .get("lifecycle")
        .or_else(|| response.pointer("/task/lifecycle"))
        .cloned()
        .unwrap_or(Value::Null);
    let db_status = response
        .get("status")
        .or_else(|| response.get("db_status"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!({
        "schema_version": 1,
        "action": action,
        "status": if db_status.is_empty() { "ok" } else { db_status },
        "message_key": format!("task_control.{action}.ok"),
        "task_id": task_id,
        "db_status": if db_status.is_empty() { Value::Null } else { json!(db_status) },
        "lifecycle": lifecycle.clone(),
        "response": response,
        "field_value": {
            "action": action,
            "status": if db_status.is_empty() { "ok" } else { db_status },
            "message_key": format!("task_control.{action}.ok"),
            "task_id": task_id,
            "db_status": if db_status.is_empty() { Value::Null } else { json!(db_status) },
            "lifecycle": lifecycle,
        },
    })
}

pub(super) fn cancel_all_result_extra(tasks: &[ActiveTaskItem], canceled: usize) -> Value {
    let items: Vec<Value> = tasks.iter().take(canceled).map(task_item_extra).collect();
    let status = if canceled == 0 { "empty" } else { "ok" };
    json!({
        "schema_version": 1,
        "action": "cancel_all",
        "status": status,
        "message_key": if canceled == 0 { "task_control.cancel_all.empty" } else { "task_control.cancel_all.ok" },
        "canceled_count": canceled,
        "requested_count": tasks.len(),
        "items": items,
        "field_value": {
            "action": "cancel_all",
            "status": status,
            "message_key": if canceled == 0 { "task_control.cancel_all.empty" } else { "task_control.cancel_all.ok" },
            "canceled_count": canceled,
            "requested_count": tasks.len(),
            "task_ids": tasks.iter().take(canceled).map(|task| task.task_id.as_str()).collect::<Vec<_>>(),
        },
    })
}

pub(super) fn cancel_one_result_extra(task: &ActiveTaskItem) -> Value {
    json!({
        "schema_version": 1,
        "action": "cancel_one",
        "status": "ok",
        "message_key": "task_control.cancel_one.ok",
        "canceled_task": task_item_extra(task),
        "field_value": {
            "action": "cancel_one",
            "status": "ok",
            "message_key": "task_control.cancel_one.ok",
            "index": task.index,
            "task_id": task.task_id,
            "db_status": task.status,
        },
    })
}

pub(super) fn task_item_extra(task: &ActiveTaskItem) -> Value {
    json!({
        "index": task.index,
        "task_id": task.task_id,
        "kind": task.kind,
        "status": task.status,
        "summary": task.summary,
        "age_seconds": task.age_seconds,
    })
}

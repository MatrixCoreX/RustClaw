use serde_json::{json, Value};

use claw_core::provider_failure_policy::ProviderFailurePolicy;

use super::{ActiveTaskItem, SkillInput};

pub(super) fn session_alias_binding_extra(alias: &str, target: &str) -> Value {
    json!({
        "schema_version": 1,
        "action": "bind_session_alias",
        "status": "ok",
        "message_key": "task_control.bind_session_alias.ok",
        "session_alias_bindings": [{
            "alias": alias,
            "target": target,
        }],
        "field_value": {
            "action": "bind_session_alias",
            "status": "ok",
            "alias": alias,
            "target": target,
        },
    })
}

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
        "dry_run": true,
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
            "dry_run": true,
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

pub(super) fn retryable_failure_observation_preview_extra() -> Value {
    json!({
        "schema_version": 1,
        "action": "preview_retryable_failure_observation",
        "status": "dry_run",
        "message_key": "task_control.preview_retryable_failure_observation.dry_run",
        "dry_run": true,
        "synthetic": true,
        "would_mutate": false,
        "observation": {
            "retryable": true,
            "error_code": "tool_retryable_failure",
            "recovery_action": "replan",
            "forbidden_repeat_signature": "[REDACTED]",
            "bounded_repair_attempts": {
                "observed_attempt_count": 1,
                "repair_attempt_count": 0,
                "max_attempts": Value::Null,
                "remaining_attempts": Value::Null,
                "limit_source": "runtime_soft_budget",
            },
        },
        "field_value": {
            "action": "preview_retryable_failure_observation",
            "status": "dry_run",
            "message_key": "task_control.preview_retryable_failure_observation.dry_run",
            "dry_run": true,
            "synthetic": true,
            "would_mutate": false,
            "retryable": true,
            "error_code": "tool_retryable_failure",
            "recovery_action": "replan",
            "forbidden_repeat_signature": "[REDACTED]",
            "bounded_repair_attempts": {
                "observed_attempt_count": 1,
                "repair_attempt_count": 0,
                "max_attempts": Value::Null,
                "remaining_attempts": Value::Null,
                "limit_source": "runtime_soft_budget",
            },
        },
    })
}

pub(super) fn repair_observation_preview_extra(repair_kind: &str) -> Option<Value> {
    let observation = match repair_kind {
        "missing_required_argument" => json!({
            "status": "validation_failed",
            "repair_source": "verifier",
            "repair_class": "loop_bounded_recovery",
            "missing_fields": ["required_argument"],
            "missing_evidence": ["required_argument"],
            "issue_codes": ["verify_missing_required_arg", "missing_required_arg"],
            "recovery_action": "replan",
            "next_recovery_kind": "replan",
            "needs_user_input": false,
        }),
        "bounded_repair_blocked" => json!({
            "status": "waiting",
            "repair_source": "runtime",
            "repair_class": "loop_bounded_recovery",
            "stop_reason_code": "repair_budget_exhausted",
            "checkpoint_id": Value::Null,
            "checkpoint_required": true,
            "needs_user_input": false,
            "evidence_refs": ["repair_signal", "attempt_ledger"],
            "recovery_action": "wait_background",
            "next_recovery_kind": "wait_background",
        }),
        _ => return None,
    };
    Some(json!({
        "schema_version": 1,
        "action": "preview_repair_observation",
        "status": "dry_run",
        "message_key": "task_control.preview_repair_observation.dry_run",
        "dry_run": true,
        "synthetic": true,
        "would_mutate": false,
        "repair_kind": repair_kind,
        "observation": observation,
        "field_value": {
            "action": "preview_repair_observation",
            "status": "dry_run",
            "message_key": "task_control.preview_repair_observation.dry_run",
            "dry_run": true,
            "synthetic": true,
            "would_mutate": false,
            "repair_kind": repair_kind,
            "observation": observation,
        },
    }))
}

pub(super) fn coding_repair_preview_extra() -> Value {
    json!({
        "schema_version": 1,
        "action": "preview_coding_repair",
        "status": "dry_run",
        "message_key": "task_control.preview_coding_repair.dry_run",
        "dry_run": true,
        "synthetic": true,
        "would_mutate": false,
        "would_execute_command": false,
        "checkpoint": {
            "status": "planned",
            "checkpoint_ref": "dry_run:checkpoint:pre_patch",
            "checkpoint_kind": "pre_patch",
            "restorable": true,
        },
        "diff": {
            "status": "planned",
            "diff_ref": "dry_run:diff:repair_patch",
            "patch_ref": "dry_run:patch:repair_attempt_1",
            "changed_file_count": 1,
        },
        "failed_verification": {
            "status": "failed",
            "verification_ref": "dry_run:verification:first",
            "evidence_ref": "dry_run:evidence:failed_verification",
            "failure_kind": "test_failure",
        },
        "repair_attempt": {
            "status": "planned",
            "attempt": 1,
            "repair_ref": "dry_run:repair:attempt_1",
            "patch_ref": "dry_run:patch:repair_attempt_1",
            "source_verification_ref": "dry_run:verification:first",
        },
        "passing_verification": {
            "status": "passed",
            "verification_ref": "dry_run:verification:second",
            "evidence_ref": "dry_run:evidence:passing_verification",
            "source_repair_ref": "dry_run:repair:attempt_1",
        },
        "rewind_references": [
            "dry_run:checkpoint:pre_patch",
            "dry_run:diff:repair_patch",
        ],
        "field_value": {
            "action": "preview_coding_repair",
            "status": "dry_run",
            "message_key": "task_control.preview_coding_repair.dry_run",
            "dry_run": true,
            "synthetic": true,
            "would_mutate": false,
            "would_execute_command": false,
            "checkpoint_ref": "dry_run:checkpoint:pre_patch",
            "diff_ref": "dry_run:diff:repair_patch",
            "failed_verification_ref": "dry_run:verification:first",
            "repair_attempt_ref": "dry_run:repair:attempt_1",
            "passing_verification_ref": "dry_run:verification:second",
            "rewind_references": [
                "dry_run:checkpoint:pre_patch",
                "dry_run:diff:repair_patch",
            ],
        },
    })
}

fn resume_preview_contract(input: &SkillInput, action: &str) -> Value {
    json!({
        "schema_version": 1,
        "action": action,
        "status": "dry_run",
        "message_key": format!("task_control.{action}.dry_run"),
        "dry_run": true,
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
            "dry_run": true,
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
        "dry_run": true,
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
            "dry_run": true,
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

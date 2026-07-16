use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::approval_grant::{ApprovalBinding, ApprovalDecision};
use crate::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskApprovalUpdate {
    pub(crate) task_id: String,
    pub(crate) request_id: String,
    pub(crate) expires_at: i64,
    pub(crate) decision: ApprovalDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskApprovalConsumeOutcome {
    Consumed,
    Missing,
    NotApproved,
    Expired,
    BindingMismatch,
    Conflict,
}

impl TaskApprovalConsumeOutcome {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Consumed => "consumed",
            Self::Missing => "missing",
            Self::NotApproved => "not_approved",
            Self::Expired => "expired",
            Self::BindingMismatch => "binding_mismatch",
            Self::Conflict => "conflict",
        }
    }

    pub(crate) fn decision_json(self, binding: &ApprovalBinding) -> Value {
        json!({
            "schema_version": 1,
            "status": self.as_token(),
            "action_fingerprint": binding.action_fingerprint,
            "arguments_hash": binding.arguments_hash,
            "action_count": binding.action_count,
        })
    }
}

pub(crate) fn decide_task_approval_request(
    state: &AppState,
    task_id: &str,
    request_id: &str,
    decision: ApprovalDecision,
) -> anyhow::Result<Option<TaskApprovalUpdate>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("db pool: {err}"))?;
    decide_task_approval_request_in_db(
        &db,
        task_id,
        request_id,
        decision,
        crate::now_ts_u64() as i64,
    )
}

pub(crate) fn consume_task_approval_grant(
    state: &AppState,
    task_id: &str,
    binding: &ApprovalBinding,
) -> anyhow::Result<TaskApprovalConsumeOutcome> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("db pool: {err}"))?;
    consume_task_approval_grant_in_db(&db, task_id, binding, crate::now_ts_u64() as i64)
}

fn decide_task_approval_request_in_db(
    db: &Connection,
    task_id: &str,
    request_id: &str,
    decision: ApprovalDecision,
    now_ts: i64,
) -> anyhow::Result<Option<TaskApprovalUpdate>> {
    let task_id = task_id.trim();
    let request_id = request_id.trim();
    if task_id.is_empty() || request_id.is_empty() {
        return Ok(None);
    }
    let record = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((status, Some(raw_result_json))) = record else {
        return Ok(None);
    };
    if status != "failed" {
        return Ok(None);
    }
    let mut result = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) if value.is_object() => value,
        _ => return Ok(None),
    };
    let Some(approval) = approval_request_mut(&mut result) else {
        return Ok(None);
    };
    if approval.get("request_id").and_then(Value::as_str) != Some(request_id)
        || approval.get("task_id").and_then(Value::as_str) != Some(task_id)
        || approval.get("status").and_then(Value::as_str) != Some("pending")
    {
        return Ok(None);
    }
    let expires_at = approval
        .get("expires_at")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if expires_at <= now_ts {
        approval.insert("status".to_string(), json!("expired"));
        approval.insert("expired_at".to_string(), json!(now_ts));
        let _ = update_approval_result_cas(db, task_id, &raw_result_json, &result, now_ts)?;
        return Ok(None);
    }
    let (request_status, reason_code, next_task_status) = match decision {
        ApprovalDecision::ApproveOnce => ("approved", "approval_grant_approved", "queued"),
        ApprovalDecision::Deny => ("denied", "approval_request_denied", "failed"),
    };
    approval.insert("status".to_string(), json!(request_status));
    approval.insert("decision".to_string(), json!(decision.as_token()));
    approval.insert("decided_at".to_string(), json!(now_ts));
    result["task_lifecycle"] = json!({
        "schema_version": 1,
        "state": if decision == ApprovalDecision::ApproveOnce { "queued" } else { "failed" },
        "reason_code": reason_code,
        "terminal_reason": if decision == ApprovalDecision::Deny {
            Value::String("approval_request_denied".to_string())
        } else {
            Value::Null
        },
        "approval_request_id": request_id,
        "approval_decision": decision.as_token(),
    });
    let changed = if decision == ApprovalDecision::ApproveOnce {
        db.execute(
            "UPDATE tasks
             SET status = ?2, result_json = ?3, error_text = NULL, updated_at = ?4,
                 lease_owner = NULL, lease_expires_at = 0, claimed_at = 0
             WHERE task_id = ?1 AND status = 'failed' AND result_json = ?5",
            params![
                task_id,
                next_task_status,
                result.to_string(),
                now_ts.to_string(),
                raw_result_json
            ],
        )?
    } else {
        db.execute(
            "UPDATE tasks SET result_json = ?2, error_text = NULL, updated_at = ?3
             WHERE task_id = ?1 AND status = 'failed' AND result_json = ?4",
            params![
                task_id,
                result.to_string(),
                now_ts.to_string(),
                raw_result_json
            ],
        )?
    };
    if changed == 0 {
        return Ok(None);
    }
    Ok(Some(TaskApprovalUpdate {
        task_id: task_id.to_string(),
        request_id: request_id.to_string(),
        expires_at,
        decision,
    }))
}

fn consume_task_approval_grant_in_db(
    db: &Connection,
    task_id: &str,
    binding: &ApprovalBinding,
    now_ts: i64,
) -> anyhow::Result<TaskApprovalConsumeOutcome> {
    let record = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((status, Some(raw_result_json))) = record else {
        return Ok(TaskApprovalConsumeOutcome::Missing);
    };
    if status != "running" {
        return Ok(TaskApprovalConsumeOutcome::NotApproved);
    }
    let mut result = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) if value.is_object() => value,
        _ => return Ok(TaskApprovalConsumeOutcome::Missing),
    };
    let Some(approval) = approval_request_mut(&mut result) else {
        return Ok(TaskApprovalConsumeOutcome::Missing);
    };
    if approval.get("task_id").and_then(Value::as_str) != Some(task_id)
        || approval.get("status").and_then(Value::as_str) != Some("approved")
    {
        return Ok(TaskApprovalConsumeOutcome::NotApproved);
    }
    if approval
        .get("expires_at")
        .and_then(Value::as_i64)
        .unwrap_or_default()
        <= now_ts
    {
        approval.insert("status".to_string(), json!("expired"));
        approval.insert("expired_at".to_string(), json!(now_ts));
        let changed = update_approval_result_cas(db, task_id, &raw_result_json, &result, now_ts)?;
        return Ok(if changed {
            TaskApprovalConsumeOutcome::Expired
        } else {
            TaskApprovalConsumeOutcome::Conflict
        });
    }
    if approval.get("action_fingerprint").and_then(Value::as_str)
        != Some(binding.action_fingerprint.as_str())
        || approval.get("arguments_hash").and_then(Value::as_str)
            != Some(binding.arguments_hash.as_str())
    {
        return Ok(TaskApprovalConsumeOutcome::BindingMismatch);
    }
    approval.insert("status".to_string(), json!("consumed"));
    approval.insert("consumed_at".to_string(), json!(now_ts));
    let changed = update_approval_result_cas(db, task_id, &raw_result_json, &result, now_ts)?;
    Ok(if changed {
        TaskApprovalConsumeOutcome::Consumed
    } else {
        TaskApprovalConsumeOutcome::Conflict
    })
}

fn approval_request_mut(result: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
    result
        .get_mut("resume_context")?
        .get_mut("approval_request")?
        .as_object_mut()
}

fn update_approval_result_cas(
    db: &Connection,
    task_id: &str,
    old_result_json: &str,
    result: &Value,
    now_ts: i64,
) -> anyhow::Result<bool> {
    Ok(db.execute(
        "UPDATE tasks SET result_json = ?2, updated_at = ?3
         WHERE task_id = ?1 AND result_json = ?4",
        params![
            task_id,
            result.to_string(),
            now_ts.to_string(),
            old_result_json
        ],
    )? > 0)
}

#[cfg(test)]
#[path = "task_approval_tests.rs"]
mod tests;

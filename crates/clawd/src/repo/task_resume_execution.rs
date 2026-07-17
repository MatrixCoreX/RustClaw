use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use crate::{AppState, ClaimedTask};

mod dispatch_claim;
mod resume_lease;

pub(crate) use dispatch_claim::record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal;
pub(crate) use dispatch_claim::{
    claim_dispatched_paused_checkpoint_resume_execution_internal,
    claim_recorded_paused_checkpoint_resume_dispatch_result_internal,
    list_dispatched_paused_checkpoint_resume_executions_internal,
    list_recorded_paused_checkpoint_resume_dispatch_results_internal,
    record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal,
    ClaimedDispatchedPausedCheckpointResumeExecution, ClaimedPausedCheckpointResumeDispatchResult,
};
pub(crate) use resume_lease::{
    merge_progress_with_active_resume_coordination,
    renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedPausedCheckpointResumeExecution {
    pub(crate) task: ClaimedTask,
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) executor_action: String,
    pub(crate) resume_trigger: String,
    pub(crate) resume_directive: String,
    pub(crate) lease_expires_at: i64,
    pub(crate) execution_plan: Value,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HandoffPausedCheckpointResumeExecution {
    pub(crate) task: ClaimedTask,
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) executor_action: String,
    pub(crate) executor_status: String,
    pub(crate) resume_trigger: String,
    pub(crate) resume_directive: String,
    pub(crate) lease_expires_at: i64,
    pub(crate) execution_plan: Value,
    pub(crate) handoff_payload: Value,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaimedHandoffPausedCheckpointResumeExecution {
    pub(crate) task: ClaimedTask,
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) executor_action: String,
    pub(crate) executor_status: String,
    pub(crate) resume_trigger: String,
    pub(crate) resume_directive: String,
    pub(crate) lease_expires_at: i64,
    pub(crate) handoff_claim_expires_at: i64,
    pub(crate) execution_plan: Value,
    pub(crate) handoff_payload: Value,
    pub(crate) handoff_claim: Value,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
}

pub(crate) fn list_planned_paused_checkpoint_resume_executions_internal(
    state: &AppState,
    now_ts: i64,
    limit: usize,
) -> anyhow::Result<Vec<PlannedPausedCheckpointResumeExecution>> {
    let limit = limit.max(1);
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, kind, payload_json, result_json
         FROM tasks
         WHERE status = 'running'
           AND result_json IS NOT NULL
         ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER) ASC,
                  task_id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            ClaimedTask {
                task_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                external_user_id: row.get(5)?,
                external_chat_id: row.get(6)?,
                kind: row.get(7)?,
                payload_json: row.get(8)?,
            },
            row.get::<_, Option<String>>(9)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (task, result_json) = row?;
        let Some(result_json) =
            result_json.and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        else {
            continue;
        };
        if let Some(planned) =
            planned_paused_checkpoint_resume_execution_from_result_json(task, &result_json, now_ts)
        {
            out.push(planned);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

pub(crate) fn list_handoff_paused_checkpoint_resume_executions_internal(
    state: &AppState,
    now_ts: i64,
    limit: usize,
) -> anyhow::Result<Vec<HandoffPausedCheckpointResumeExecution>> {
    let limit = limit.max(1);
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut stmt = db.prepare(
        "SELECT task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, kind, payload_json, result_json
         FROM tasks
         WHERE status = 'running'
           AND result_json IS NOT NULL
         ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at, '0') AS INTEGER) ASC,
                  task_id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            ClaimedTask {
                task_id: row.get(0)?,
                user_id: row.get(1)?,
                chat_id: row.get(2)?,
                user_key: row.get(3)?,
                channel: row.get(4)?,
                external_user_id: row.get(5)?,
                external_chat_id: row.get(6)?,
                kind: row.get(7)?,
                payload_json: row.get(8)?,
            },
            row.get::<_, Option<String>>(9)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (task, result_json) = row?;
        let Some(result_json) =
            result_json.and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        else {
            continue;
        };
        if let Some(handoff) =
            handoff_paused_checkpoint_resume_execution_from_result_json(task, &result_json, now_ts)
        {
            out.push(handoff);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

pub(crate) fn record_planned_paused_checkpoint_resume_handoff_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    handoff_payload: &Value,
    now_ts: i64,
) -> anyhow::Result<bool> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let executor_state = executor_state.trim();
    let executor_action = executor_action.trim();
    if task_id.is_empty()
        || checkpoint_id.is_empty()
        || executor_state.is_empty()
        || executor_action.is_empty()
        || !handoff_payload.is_object()
        || handoff_payload.get("text").is_some()
        || handoff_payload.get("error_text").is_some()
    {
        return Ok(false);
    }
    let handoff_checkpoint_id = handoff_payload
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let handoff_executor_state = handoff_payload
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let handoff_executor_action = handoff_payload
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let handoff_status = handoff_payload
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if handoff_checkpoint_id != checkpoint_id
        || handoff_executor_state != executor_state
        || handoff_executor_action != executor_action
        || !matches!(
            handoff_status,
            "seeded_loop_requires_provider_window"
                | "async_poll_adapter_pending"
                | "checkpoint_finalize_executor_pending"
        )
    {
        return Ok(false);
    }

    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let raw_result_json = db
        .query_row(
            "SELECT result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            rusqlite::params![task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(raw_result_json) = raw_result_json else {
        return Ok(false);
    };
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    if planned_paused_checkpoint_resume_execution_from_result_json(
        ClaimedTask {
            task_id: task_id.to_string(),
            user_id: 0,
            chat_id: 0,
            user_key: None,
            channel: String::new(),
            external_user_id: None,
            external_chat_id: None,
            kind: String::new(),
            payload_json: String::new(),
        },
        &result_json,
        now_ts,
    )
    .filter(|planned| {
        planned.checkpoint_id == checkpoint_id
            && planned.executor_state == executor_state
            && planned.executor_action == executor_action
    })
    .is_none()
    {
        return Ok(false);
    }
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(false);
    };
    let mut handoff = handoff_payload.clone();
    if let Some(handoff_obj) = handoff.as_object_mut() {
        handoff_obj.insert("handoff_at".to_string(), serde_json::json!(now_ts));
    }
    obj.insert("resume_executor_handoff".to_string(), handoff);
    if let Some(executor_obj) = obj
        .get_mut("resume_executor")
        .and_then(serde_json::Value::as_object_mut)
    {
        executor_obj.insert(
            "executor_status".to_string(),
            serde_json::json!(handoff_status),
        );
        executor_obj.insert("handoff_at".to_string(), serde_json::json!(now_ts));
    }
    if let Some(claim_obj) = obj
        .get_mut("resume_executor_claim")
        .and_then(serde_json::Value::as_object_mut)
    {
        claim_obj.insert(
            "executor_status".to_string(),
            serde_json::json!(handoff_status),
        );
        claim_obj.insert("handoff_at".to_string(), serde_json::json!(now_ts));
    }

    result_json["task_lifecycle"] = lifecycle;
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4",
        rusqlite::params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json
        ],
    )?;
    Ok(changed > 0)
}

pub(crate) fn claim_handoff_paused_checkpoint_resume_execution_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    now_ts: i64,
    lease_seconds: i64,
) -> anyhow::Result<Option<ClaimedHandoffPausedCheckpointResumeExecution>> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let executor_state = executor_state.trim();
    let executor_action = executor_action.trim();
    let executor_status = executor_status.trim();
    if task_id.is_empty()
        || checkpoint_id.is_empty()
        || executor_state.is_empty()
        || executor_action.is_empty()
        || !allowed_resume_executor_handoff_status(executor_status)
    {
        return Ok(None);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let task_row = db
        .query_row(
            "SELECT task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, kind, payload_json, result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            params![task_id],
            |row| {
                Ok((
                    ClaimedTask {
                        task_id: row.get(0)?,
                        user_id: row.get(1)?,
                        chat_id: row.get(2)?,
                        user_key: row.get(3)?,
                        channel: row.get(4)?,
                        external_user_id: row.get(5)?,
                        external_chat_id: row.get(6)?,
                        kind: row.get(7)?,
                        payload_json: row.get(8)?,
                    },
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((task, raw_result_json)) = task_row else {
        return Ok(None);
    };
    let Some(raw_result_json) = raw_result_json else {
        return Ok(None);
    };
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let Some(handoff) = handoff_paused_checkpoint_resume_execution_from_result_json(
        task.clone(),
        &result_json,
        now_ts,
    ) else {
        return Ok(None);
    };
    if handoff.checkpoint_id != checkpoint_id
        || handoff.executor_state != executor_state
        || handoff.executor_action != executor_action
        || handoff.executor_status != executor_status
    {
        return Ok(None);
    }

    let handoff_claim_expires_at = now_ts
        .saturating_add(lease_seconds.max(1))
        .min(handoff.lease_expires_at);
    if handoff_claim_expires_at <= now_ts {
        return Ok(None);
    }
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(None);
    };
    let claim_payload = serde_json::json!({
        "schema_version": 1,
        "owner": "worker_recovery_handoff_executor",
        "checkpoint_id": checkpoint_id,
        "executor_state": executor_state,
        "executor_action": executor_action,
        "executor_status": executor_status,
        "claimed_at": now_ts,
        "expires_at": handoff_claim_expires_at
    });
    obj.insert(
        "resume_executor_handoff_claim".to_string(),
        claim_payload.clone(),
    );
    if let Some(handoff_obj) = obj
        .get_mut("resume_executor_handoff")
        .and_then(serde_json::Value::as_object_mut)
    {
        handoff_obj.insert("claim_state".to_string(), serde_json::json!("claimed"));
        handoff_obj.insert(
            "claim_owner".to_string(),
            serde_json::json!("worker_recovery_handoff_executor"),
        );
        handoff_obj.insert("claimed_at".to_string(), serde_json::json!(now_ts));
        handoff_obj.insert(
            "claim_expires_at".to_string(),
            serde_json::json!(handoff_claim_expires_at),
        );
    }
    if let Some(executor_obj) = obj
        .get_mut("resume_executor")
        .and_then(serde_json::Value::as_object_mut)
    {
        executor_obj.insert("handoff_claimed_at".to_string(), serde_json::json!(now_ts));
        executor_obj.insert(
            "handoff_claim_expires_at".to_string(),
            serde_json::json!(handoff_claim_expires_at),
        );
    }
    if let Some(claim_obj) = obj
        .get_mut("resume_executor_claim")
        .and_then(serde_json::Value::as_object_mut)
    {
        claim_obj.insert("handoff_claimed_at".to_string(), serde_json::json!(now_ts));
        claim_obj.insert(
            "handoff_claim_expires_at".to_string(),
            serde_json::json!(handoff_claim_expires_at),
        );
    }

    result_json["task_lifecycle"] = lifecycle;
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json
        ],
    )?;
    if changed == 0 {
        return Ok(None);
    }

    Ok(Some(ClaimedHandoffPausedCheckpointResumeExecution {
        task,
        task_id: handoff.task_id,
        checkpoint_id: handoff.checkpoint_id,
        executor_state: handoff.executor_state,
        executor_action: handoff.executor_action,
        executor_status: handoff.executor_status,
        resume_trigger: handoff.resume_trigger,
        resume_directive: handoff.resume_directive,
        lease_expires_at: handoff.lease_expires_at,
        handoff_claim_expires_at,
        execution_plan: handoff.execution_plan,
        handoff_payload: handoff.handoff_payload,
        handoff_claim: claim_payload,
        task_checkpoint: handoff.task_checkpoint,
    }))
}

pub(crate) fn record_claimed_handoff_paused_checkpoint_resume_dispatch_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_payload: &Value,
    now_ts: i64,
) -> anyhow::Result<bool> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let executor_state = executor_state.trim();
    let executor_action = executor_action.trim();
    let executor_status = executor_status.trim();
    if task_id.is_empty()
        || checkpoint_id.is_empty()
        || executor_state.is_empty()
        || executor_action.is_empty()
        || !allowed_resume_executor_handoff_status(executor_status)
        || !dispatch_payload.is_object()
        || dispatch_payload.get("text").is_some()
        || dispatch_payload.get("error_text").is_some()
    {
        return Ok(false);
    }
    let dispatch_checkpoint_id = dispatch_payload
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let dispatch_executor_state = dispatch_payload
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let dispatch_executor_action = dispatch_payload
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let dispatch_executor_status = dispatch_payload
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let dispatch_state = dispatch_payload
        .get("dispatch_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if dispatch_checkpoint_id != checkpoint_id
        || dispatch_executor_state != executor_state
        || dispatch_executor_action != executor_action
        || dispatch_executor_status != executor_status
        || !allowed_resume_executor_handoff_dispatch_state(
            executor_action,
            executor_status,
            dispatch_state,
        )
    {
        return Ok(false);
    }

    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let raw_result_json = db
        .query_row(
            "SELECT result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            params![task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(raw_result_json) = raw_result_json else {
        return Ok(false);
    };
    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(false);
    };
    if matching_resume_executor_handoff(obj, checkpoint_id, executor_state, executor_action)
        .filter(|handoff| {
            handoff
                .get("executor_status")
                .and_then(Value::as_str)
                .map(str::trim)
                == Some(executor_status)
        })
        .is_none()
    {
        return Ok(false);
    }
    if !active_resume_executor_handoff_claim(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        now_ts,
    ) {
        return Ok(false);
    }
    let mut dispatch = dispatch_payload.clone();
    if let Some(dispatch_obj) = dispatch.as_object_mut() {
        dispatch_obj.insert("dispatched_at".to_string(), serde_json::json!(now_ts));
    }
    obj.insert("resume_executor_handoff_dispatch".to_string(), dispatch);
    if let Some(handoff_obj) = obj
        .get_mut("resume_executor_handoff")
        .and_then(serde_json::Value::as_object_mut)
    {
        handoff_obj.insert(
            "dispatch_state".to_string(),
            serde_json::json!(dispatch_state),
        );
        handoff_obj.insert("dispatched_at".to_string(), serde_json::json!(now_ts));
    }
    if let Some(executor_obj) = obj
        .get_mut("resume_executor")
        .and_then(serde_json::Value::as_object_mut)
    {
        executor_obj.insert(
            "dispatch_state".to_string(),
            serde_json::json!(dispatch_state),
        );
        executor_obj.insert("dispatched_at".to_string(), serde_json::json!(now_ts));
    }
    if let Some(claim_obj) = obj
        .get_mut("resume_executor_claim")
        .and_then(serde_json::Value::as_object_mut)
    {
        claim_obj.insert(
            "dispatch_state".to_string(),
            serde_json::json!(dispatch_state),
        );
        claim_obj.insert("dispatched_at".to_string(), serde_json::json!(now_ts));
    }

    result_json["task_lifecycle"] = lifecycle;
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?4",
        params![
            task_id,
            updated_result_json,
            now_ts.to_string(),
            raw_result_json
        ],
    )?;
    Ok(changed > 0)
}

fn planned_paused_checkpoint_resume_execution_from_result_json(
    task: ClaimedTask,
    result_json: &Value,
    now_ts: i64,
) -> Option<PlannedPausedCheckpointResumeExecution> {
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(result_json), None);
    let lifecycle_obj = lifecycle.as_object()?;
    if lifecycle_obj
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        != Some("running")
    {
        return None;
    }
    let checkpoint_id = lifecycle_obj
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let task_checkpoint = crate::task_lifecycle::task_checkpoint_from_result_json(result_json)?;
    if task_checkpoint.checkpoint_id != checkpoint_id {
        return None;
    }
    let claim = lifecycle_obj.get("resume_executor_claim")?;
    let claim_checkpoint_id = claim
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id {
        return None;
    }
    let executor_state = claim
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let lease_expires_at = claim.get("expires_at").and_then(Value::as_i64)?;
    if lease_expires_at <= now_ts {
        return None;
    }
    let execution_plan = lifecycle_obj
        .get("resume_execution_plan")
        .filter(|value| value.is_object())?;
    if execution_plan.get("text").is_some() || execution_plan.get("error_text").is_some() {
        return None;
    }
    let plan_checkpoint_id = execution_plan
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let plan_executor_state = execution_plan
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if plan_checkpoint_id != checkpoint_id || plan_executor_state != executor_state {
        return None;
    }
    let executor_action = execution_plan
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            matches!(
                *value,
                "run_seeded_agent_loop" | "poll_async_job" | "verify_and_finalize"
            )
        })?;
    if let Some(plan_task_id) = execution_plan
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::trim)
    {
        if plan_task_id != task.task_id {
            return None;
        }
    }
    let resume_executor = lifecycle_obj
        .get("resume_executor")
        .filter(|value| value.is_object())?;
    let resume_directive = resume_executor
        .get("resume_directive")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let resume_trigger = resume_executor
        .get("resume_trigger")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("unknown");
    if let Some(recorded_action) = resume_executor
        .get("execution_plan_action")
        .and_then(Value::as_str)
        .map(str::trim)
    {
        if recorded_action != executor_action {
            return None;
        }
    }
    if matching_resume_executor_handoff(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
    )
    .is_some()
    {
        return None;
    }
    Some(PlannedPausedCheckpointResumeExecution {
        task_id: task.task_id.clone(),
        task,
        checkpoint_id: checkpoint_id.to_string(),
        executor_state: executor_state.to_string(),
        executor_action: executor_action.to_string(),
        resume_trigger: resume_trigger.to_string(),
        resume_directive: resume_directive.to_string(),
        lease_expires_at,
        execution_plan: execution_plan.clone(),
        task_checkpoint,
    })
}

fn handoff_paused_checkpoint_resume_execution_from_result_json(
    task: ClaimedTask,
    result_json: &Value,
    now_ts: i64,
) -> Option<HandoffPausedCheckpointResumeExecution> {
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(result_json), None);
    let lifecycle_obj = lifecycle.as_object()?;
    if lifecycle_obj
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        != Some("running")
    {
        return None;
    }
    let checkpoint_id = lifecycle_obj
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let task_checkpoint = crate::task_lifecycle::task_checkpoint_from_result_json(result_json)?;
    if task_checkpoint.checkpoint_id != checkpoint_id {
        return None;
    }
    let claim = lifecycle_obj.get("resume_executor_claim")?;
    let claim_checkpoint_id = claim
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if claim_checkpoint_id != checkpoint_id {
        return None;
    }
    let executor_state = claim
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let lease_expires_at = claim.get("expires_at").and_then(Value::as_i64)?;
    if lease_expires_at <= now_ts {
        return None;
    }
    let execution_plan = lifecycle_obj
        .get("resume_execution_plan")
        .filter(|value| value.is_object())?;
    if execution_plan.get("text").is_some() || execution_plan.get("error_text").is_some() {
        return None;
    }
    let executor_action = execution_plan
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            matches!(
                *value,
                "run_seeded_agent_loop" | "poll_async_job" | "verify_and_finalize"
            )
        })?;
    let plan_checkpoint_id = execution_plan
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let plan_executor_state = execution_plan
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if plan_checkpoint_id != checkpoint_id || plan_executor_state != executor_state {
        return None;
    }
    if let Some(plan_task_id) = execution_plan
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::trim)
    {
        if plan_task_id != task.task_id {
            return None;
        }
    }
    let handoff_payload = matching_resume_executor_handoff(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
    )?;
    let executor_status = handoff_payload
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if active_resume_executor_handoff_claim(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        now_ts,
    ) {
        return None;
    }
    let resume_executor = lifecycle_obj
        .get("resume_executor")
        .filter(|value| value.is_object())?;
    let resume_directive = resume_executor
        .get("resume_directive")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let resume_trigger = resume_executor
        .get("resume_trigger")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("unknown");
    Some(HandoffPausedCheckpointResumeExecution {
        task_id: task.task_id.clone(),
        task,
        checkpoint_id: checkpoint_id.to_string(),
        executor_state: executor_state.to_string(),
        executor_action: executor_action.to_string(),
        executor_status: executor_status.to_string(),
        resume_trigger: resume_trigger.to_string(),
        resume_directive: resume_directive.to_string(),
        lease_expires_at,
        execution_plan: execution_plan.clone(),
        handoff_payload: handoff_payload.clone(),
        task_checkpoint,
    })
}

fn matching_resume_executor_handoff<'a>(
    lifecycle_obj: &'a serde_json::Map<String, Value>,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
) -> Option<&'a Value> {
    let handoff = lifecycle_obj
        .get("resume_executor_handoff")
        .filter(|value| value.is_object())?;
    if handoff.get("text").is_some() || handoff.get("error_text").is_some() {
        return None;
    }
    let handoff_checkpoint_id = handoff
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let handoff_executor_state = handoff
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let handoff_executor_action = handoff
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let handoff_status = handoff
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if handoff_checkpoint_id == checkpoint_id
        && handoff_executor_state == executor_state
        && handoff_executor_action == executor_action
        && allowed_resume_executor_handoff_status(handoff_status)
    {
        Some(handoff)
    } else {
        None
    }
}

fn active_resume_executor_handoff_claim(
    lifecycle_obj: &serde_json::Map<String, Value>,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    now_ts: i64,
) -> bool {
    let Some(claim) = lifecycle_obj
        .get("resume_executor_handoff_claim")
        .filter(|value| value.is_object())
    else {
        return false;
    };
    if claim.get("text").is_some() || claim.get("error_text").is_some() {
        return true;
    }
    let claim_checkpoint_id = claim
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let claim_executor_state = claim
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let claim_executor_action = claim
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let claim_executor_status = claim
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let expires_at = claim.get("expires_at").and_then(Value::as_i64).unwrap_or(0);
    claim_checkpoint_id == checkpoint_id
        && claim_executor_state == executor_state
        && claim_executor_action == executor_action
        && claim_executor_status == executor_status
        && expires_at > now_ts
}

fn allowed_resume_executor_handoff_status(status: &str) -> bool {
    matches!(
        status,
        "seeded_loop_requires_provider_window"
            | "async_poll_adapter_pending"
            | "checkpoint_finalize_executor_pending"
    )
}

fn allowed_resume_executor_handoff_dispatch_state(
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
) -> bool {
    matches!(
        (executor_action, executor_status, dispatch_state),
        (
            "run_seeded_agent_loop",
            "seeded_loop_requires_provider_window",
            "ready_to_run_seeded_agent_loop"
        ) | (
            "poll_async_job",
            "async_poll_adapter_pending",
            "ready_to_poll_async_job"
        ) | (
            "verify_and_finalize",
            "checkpoint_finalize_executor_pending",
            "ready_to_verify_and_finalize"
        )
    )
}

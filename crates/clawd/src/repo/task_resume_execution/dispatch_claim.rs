use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use crate::{AppState, ClaimedTask};

#[path = "result_projection.rs"]
mod result_projection;

pub(crate) use result_projection::{
    claim_recorded_paused_checkpoint_resume_dispatch_result_internal,
    list_recorded_paused_checkpoint_resume_dispatch_results_internal,
    record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal,
    ClaimedPausedCheckpointResumeDispatchResult,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DispatchedPausedCheckpointResumeExecution {
    pub(crate) task: ClaimedTask,
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) executor_action: String,
    pub(crate) executor_status: String,
    pub(crate) dispatch_state: String,
    pub(crate) dispatch_execution_state: String,
    pub(crate) resume_trigger: String,
    pub(crate) resume_directive: String,
    pub(crate) lease_expires_at: i64,
    pub(crate) handoff_claim_expires_at: i64,
    pub(crate) execution_plan: Value,
    pub(crate) dispatch_payload: Value,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaimedDispatchedPausedCheckpointResumeExecution {
    pub(crate) task: ClaimedTask,
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) executor_action: String,
    pub(crate) executor_status: String,
    pub(crate) dispatch_state: String,
    pub(crate) dispatch_execution_state: String,
    pub(crate) resume_trigger: String,
    pub(crate) resume_directive: String,
    pub(crate) lease_expires_at: i64,
    pub(crate) handoff_claim_expires_at: i64,
    pub(crate) dispatch_claim_expires_at: i64,
    pub(crate) execution_plan: Value,
    pub(crate) dispatch_payload: Value,
    pub(crate) dispatch_claim: Value,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
}

pub(crate) fn list_dispatched_paused_checkpoint_resume_executions_internal(
    state: &AppState,
    now_ts: i64,
    limit: usize,
) -> anyhow::Result<Vec<DispatchedPausedCheckpointResumeExecution>> {
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
        if let Some(dispatched) = dispatched_paused_checkpoint_resume_execution_from_result_json(
            task,
            &result_json,
            &state.worker.worker_id,
            now_ts,
        ) {
            out.push(dispatched);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

pub(crate) fn claim_dispatched_paused_checkpoint_resume_execution_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
    now_ts: i64,
    lease_seconds: i64,
) -> anyhow::Result<Option<ClaimedDispatchedPausedCheckpointResumeExecution>> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let executor_state = executor_state.trim();
    let executor_action = executor_action.trim();
    let executor_status = executor_status.trim();
    let dispatch_state = dispatch_state.trim();
    if task_id.is_empty()
        || checkpoint_id.is_empty()
        || executor_state.is_empty()
        || executor_action.is_empty()
        || !super::allowed_resume_executor_handoff_status(executor_status)
        || !super::allowed_resume_executor_handoff_dispatch_state(
            executor_action,
            executor_status,
            dispatch_state,
        )
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
    let Some(dispatched) = dispatched_paused_checkpoint_resume_execution_from_result_json(
        task.clone(),
        &result_json,
        &state.worker.worker_id,
        now_ts,
    ) else {
        return Ok(None);
    };
    if dispatched.checkpoint_id != checkpoint_id
        || dispatched.executor_state != executor_state
        || dispatched.executor_action != executor_action
        || dispatched.executor_status != executor_status
        || dispatched.dispatch_state != dispatch_state
    {
        return Ok(None);
    }

    let dispatch_claim_expires_at = now_ts
        .saturating_add(lease_seconds.max(1))
        .min(dispatched.handoff_claim_expires_at)
        .min(dispatched.lease_expires_at);
    if dispatch_claim_expires_at <= now_ts {
        return Ok(None);
    }
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(None);
    };
    let dispatch_claim = serde_json::json!({
        "schema_version": 1,
        "owner": "worker_recovery_dispatch_executor",
        "checkpoint_id": checkpoint_id,
        "executor_state": executor_state,
        "executor_action": executor_action,
        "executor_status": executor_status,
        "dispatch_state": dispatch_state,
        "dispatch_execution_state": dispatched.dispatch_execution_state.as_str(),
        "claimed_at": now_ts,
        "expires_at": dispatch_claim_expires_at
    });
    obj.insert(
        "resume_executor_dispatch_claim".to_string(),
        dispatch_claim.clone(),
    );
    if let Some(dispatch_obj) = obj
        .get_mut("resume_executor_handoff_dispatch")
        .and_then(serde_json::Value::as_object_mut)
    {
        dispatch_obj.insert("claim_state".to_string(), serde_json::json!("claimed"));
        dispatch_obj.insert(
            "claim_owner".to_string(),
            serde_json::json!("worker_recovery_dispatch_executor"),
        );
        dispatch_obj.insert("claimed_at".to_string(), serde_json::json!(now_ts));
        dispatch_obj.insert(
            "claim_expires_at".to_string(),
            serde_json::json!(dispatch_claim_expires_at),
        );
        dispatch_obj.insert(
            "dispatch_execution_state".to_string(),
            serde_json::json!(dispatched.dispatch_execution_state.as_str()),
        );
    }
    for key in [
        "resume_executor",
        "resume_executor_claim",
        "resume_executor_handoff",
        "resume_executor_handoff_claim",
    ] {
        if let Some(target_obj) = obj.get_mut(key).and_then(serde_json::Value::as_object_mut) {
            target_obj.insert("dispatch_claimed_at".to_string(), serde_json::json!(now_ts));
            target_obj.insert(
                "dispatch_claim_expires_at".to_string(),
                serde_json::json!(dispatch_claim_expires_at),
            );
            target_obj.insert(
                "dispatch_execution_state".to_string(),
                serde_json::json!(dispatched.dispatch_execution_state.as_str()),
            );
        }
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

    Ok(Some(ClaimedDispatchedPausedCheckpointResumeExecution {
        task,
        task_id: dispatched.task_id,
        checkpoint_id: dispatched.checkpoint_id,
        executor_state: dispatched.executor_state,
        executor_action: dispatched.executor_action,
        executor_status: dispatched.executor_status,
        dispatch_state: dispatched.dispatch_state,
        dispatch_execution_state: dispatched.dispatch_execution_state,
        resume_trigger: dispatched.resume_trigger,
        resume_directive: dispatched.resume_directive,
        lease_expires_at: dispatched.lease_expires_at,
        handoff_claim_expires_at: dispatched.handoff_claim_expires_at,
        dispatch_claim_expires_at,
        execution_plan: dispatched.execution_plan,
        dispatch_payload: dispatched.dispatch_payload,
        dispatch_claim,
        task_checkpoint: dispatched.task_checkpoint,
    }))
}

pub(crate) fn record_claimed_dispatched_paused_checkpoint_resume_execution_result_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
    execution_result_payload: &Value,
    now_ts: i64,
) -> anyhow::Result<bool> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let executor_state = executor_state.trim();
    let executor_action = executor_action.trim();
    let executor_status = executor_status.trim();
    let dispatch_state = dispatch_state.trim();
    if task_id.is_empty()
        || checkpoint_id.is_empty()
        || executor_state.is_empty()
        || executor_action.is_empty()
        || !super::allowed_resume_executor_handoff_status(executor_status)
        || !super::allowed_resume_executor_handoff_dispatch_state(
            executor_action,
            executor_status,
            dispatch_state,
        )
        || !execution_result_payload.is_object()
        || execution_result_payload.get("text").is_some()
        || execution_result_payload.get("error_text").is_some()
    {
        return Ok(false);
    }
    let payload_checkpoint_id = execution_result_payload
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let payload_executor_state = execution_result_payload
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let payload_executor_action = execution_result_payload
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let payload_executor_status = execution_result_payload
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let payload_dispatch_state = execution_result_payload
        .get("dispatch_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let executor_result_status = execution_result_payload
        .get("executor_result_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if payload_checkpoint_id != checkpoint_id
        || payload_executor_state != executor_state
        || payload_executor_action != executor_action
        || payload_executor_status != executor_status
        || payload_dispatch_state != dispatch_state
        || !allowed_dispatch_execution_result_status(executor_action, executor_result_status)
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
    if super::matching_resume_executor_handoff(obj, checkpoint_id, executor_state, executor_action)
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
    if matching_resume_executor_handoff_dispatch(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
    )
    .and_then(|dispatch| dispatch.get("dispatch_state").and_then(Value::as_str))
    .map(str::trim)
        != Some(dispatch_state)
    {
        return Ok(false);
    }
    if !active_resume_executor_dispatch_claim(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
        now_ts,
    ) {
        return Ok(false);
    }

    let mut result = execution_result_payload.clone();
    if let Some(result_obj) = result.as_object_mut() {
        result_obj.insert("recorded_at".to_string(), serde_json::json!(now_ts));
        result_obj.insert(
            "projection_pending_reason".to_string(),
            serde_json::json!(projection_pending_reason(
                executor_action,
                executor_result_status
            )),
        );
    }
    obj.insert("resume_executor_dispatch_result".to_string(), result);
    for key in [
        "resume_executor_handoff_dispatch",
        "resume_executor_dispatch_claim",
        "resume_executor",
        "resume_executor_claim",
        "resume_executor_handoff",
        "resume_executor_handoff_claim",
    ] {
        if let Some(target_obj) = obj.get_mut(key).and_then(serde_json::Value::as_object_mut) {
            target_obj.insert(
                "executor_result_status".to_string(),
                serde_json::json!(executor_result_status),
            );
            target_obj.insert("executor_result_at".to_string(), serde_json::json!(now_ts));
        }
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

fn dispatched_paused_checkpoint_resume_execution_from_result_json(
    task: ClaimedTask,
    result_json: &Value,
    expected_worker_id: &str,
    now_ts: i64,
) -> Option<DispatchedPausedCheckpointResumeExecution> {
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
    if !super::active_resume_claim_owner(lifecycle_obj, checkpoint_id, expected_worker_id, now_ts) {
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
    let handoff_payload = super::matching_resume_executor_handoff(
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
    let handoff_claim = lifecycle_obj
        .get("resume_executor_handoff_claim")
        .filter(|value| value.is_object())?;
    if !super::active_resume_executor_handoff_claim(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        now_ts,
    ) {
        return None;
    }
    let handoff_claim_expires_at = handoff_claim.get("expires_at").and_then(Value::as_i64)?;
    let dispatch_payload = matching_resume_executor_handoff_dispatch(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
    )?;
    let dispatch_state = dispatch_payload
        .get("dispatch_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if active_resume_executor_dispatch_claim(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
        now_ts,
    ) {
        return None;
    }
    if matching_resume_executor_dispatch_result(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
    )
    .is_some()
    {
        return None;
    }
    let dispatch_execution_state = resume_executor_dispatch_execution_state(dispatch_state)?;
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
    Some(DispatchedPausedCheckpointResumeExecution {
        task_id: task.task_id.clone(),
        task,
        checkpoint_id: checkpoint_id.to_string(),
        executor_state: executor_state.to_string(),
        executor_action: executor_action.to_string(),
        executor_status: executor_status.to_string(),
        dispatch_state: dispatch_state.to_string(),
        dispatch_execution_state: dispatch_execution_state.to_string(),
        resume_trigger: resume_trigger.to_string(),
        resume_directive: resume_directive.to_string(),
        lease_expires_at,
        handoff_claim_expires_at,
        execution_plan: execution_plan.clone(),
        dispatch_payload: dispatch_payload.clone(),
        task_checkpoint,
    })
}

pub(super) fn matching_resume_executor_dispatch_result<'a>(
    lifecycle_obj: &'a serde_json::Map<String, Value>,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
) -> Option<&'a Value> {
    let result = lifecycle_obj
        .get("resume_executor_dispatch_result")
        .filter(|value| value.is_object())?;
    if result.get("text").is_some() || result.get("error_text").is_some() {
        return Some(result);
    }
    let result_checkpoint_id = result
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let result_executor_state = result
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let result_executor_action = result
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let result_executor_status = result
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let result_dispatch_state = result
        .get("dispatch_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let executor_result_status = result
        .get("executor_result_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if result_checkpoint_id == checkpoint_id
        && result_executor_state == executor_state
        && result_executor_action == executor_action
        && result_executor_status == executor_status
        && result_dispatch_state == dispatch_state
        && allowed_dispatch_execution_result_status(executor_action, executor_result_status)
    {
        Some(result)
    } else {
        None
    }
}

pub(super) fn matching_resume_executor_handoff_dispatch<'a>(
    lifecycle_obj: &'a serde_json::Map<String, Value>,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
) -> Option<&'a Value> {
    let dispatch = lifecycle_obj
        .get("resume_executor_handoff_dispatch")
        .filter(|value| value.is_object())?;
    if dispatch.get("text").is_some() || dispatch.get("error_text").is_some() {
        return None;
    }
    let dispatch_checkpoint_id = dispatch
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let dispatch_executor_state = dispatch
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let dispatch_executor_action = dispatch
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let dispatch_executor_status = dispatch
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let dispatch_state = dispatch
        .get("dispatch_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if dispatch_checkpoint_id == checkpoint_id
        && dispatch_executor_state == executor_state
        && dispatch_executor_action == executor_action
        && dispatch_executor_status == executor_status
        && super::allowed_resume_executor_handoff_dispatch_state(
            executor_action,
            executor_status,
            dispatch_state,
        )
    {
        Some(dispatch)
    } else {
        None
    }
}

pub(super) fn allowed_dispatch_execution_result_status(
    executor_action: &str,
    executor_result_status: &str,
) -> bool {
    matches!(
        (executor_action, executor_result_status),
        ("run_seeded_agent_loop", "seeded_loop_completed")
            | ("run_seeded_agent_loop", "seeded_loop_deferred")
            | ("run_seeded_agent_loop", "seeded_loop_failed")
            | ("poll_async_job", "async_poll_completed")
            | ("poll_async_job", "async_poll_rescheduled")
            | ("poll_async_job", "async_poll_failed")
            | ("poll_async_job", "async_poll_cancelled")
            | ("verify_and_finalize", "finalize_completed")
            | ("verify_and_finalize", "finalize_failed")
    )
}

fn projection_pending_reason(executor_action: &str, executor_result_status: &str) -> &'static str {
    if terminal_dispatch_execution_result_status(executor_action, executor_result_status) {
        "terminal_projection_pending"
    } else {
        "result_projection_pending"
    }
}

fn terminal_dispatch_execution_result_status(
    executor_action: &str,
    executor_result_status: &str,
) -> bool {
    matches!(
        (executor_action, executor_result_status),
        ("run_seeded_agent_loop", "seeded_loop_completed")
            | ("run_seeded_agent_loop", "seeded_loop_failed")
            | ("poll_async_job", "async_poll_completed")
            | ("poll_async_job", "async_poll_failed")
            | ("poll_async_job", "async_poll_cancelled")
            | ("verify_and_finalize", "finalize_completed")
            | ("verify_and_finalize", "finalize_failed")
    )
}

fn active_resume_executor_dispatch_claim(
    lifecycle_obj: &serde_json::Map<String, Value>,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
    now_ts: i64,
) -> bool {
    let Some(claim) = lifecycle_obj
        .get("resume_executor_dispatch_claim")
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
    let claim_dispatch_state = claim
        .get("dispatch_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let expires_at = claim.get("expires_at").and_then(Value::as_i64).unwrap_or(0);
    claim_checkpoint_id == checkpoint_id
        && claim_executor_state == executor_state
        && claim_executor_action == executor_action
        && claim_executor_status == executor_status
        && claim_dispatch_state == dispatch_state
        && expires_at > now_ts
}

fn resume_executor_dispatch_execution_state(dispatch_state: &str) -> Option<&'static str> {
    match dispatch_state {
        "ready_to_run_seeded_agent_loop" => Some("claimed_to_run_seeded_agent_loop"),
        "ready_to_poll_async_job" => Some("claimed_to_poll_async_job"),
        "ready_to_verify_and_finalize" => Some("claimed_to_verify_and_finalize"),
        _ => None,
    }
}

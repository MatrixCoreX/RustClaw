use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};

use crate::{AppState, ClaimedTask};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecordedPausedCheckpointResumeDispatchResult {
    pub(crate) task: ClaimedTask,
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) executor_action: String,
    pub(crate) executor_status: String,
    pub(crate) dispatch_state: String,
    pub(crate) executor_result_status: String,
    pub(crate) result_projection_state: String,
    pub(crate) recorded_at: i64,
    pub(crate) execution_result_payload: Value,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaimedPausedCheckpointResumeDispatchResult {
    pub(crate) task: ClaimedTask,
    pub(crate) task_id: String,
    pub(crate) checkpoint_id: String,
    pub(crate) executor_state: String,
    pub(crate) executor_action: String,
    pub(crate) executor_status: String,
    pub(crate) dispatch_state: String,
    pub(crate) executor_result_status: String,
    pub(crate) result_projection_state: String,
    pub(crate) recorded_at: i64,
    pub(crate) result_projection_claim_expires_at: i64,
    pub(crate) execution_result_payload: Value,
    pub(crate) result_projection_claim: Value,
    pub(crate) task_checkpoint: crate::task_lifecycle::TaskCheckpoint,
}

pub(crate) fn list_recorded_paused_checkpoint_resume_dispatch_results_internal(
    state: &AppState,
    now_ts: i64,
    limit: usize,
) -> anyhow::Result<Vec<RecordedPausedCheckpointResumeDispatchResult>> {
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
        if let Some(recorded) = recorded_paused_checkpoint_resume_dispatch_result_from_result_json(
            task,
            &result_json,
            now_ts,
        ) {
            out.push(recorded);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

pub(crate) fn claim_recorded_paused_checkpoint_resume_dispatch_result_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
    executor_result_status: &str,
    now_ts: i64,
    lease_seconds: i64,
) -> anyhow::Result<Option<ClaimedPausedCheckpointResumeDispatchResult>> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let executor_state = executor_state.trim();
    let executor_action = executor_action.trim();
    let executor_status = executor_status.trim();
    let dispatch_state = dispatch_state.trim();
    let executor_result_status = executor_result_status.trim();
    if task_id.is_empty()
        || checkpoint_id.is_empty()
        || executor_state.is_empty()
        || executor_action.is_empty()
        || !super::super::allowed_resume_executor_handoff_status(executor_status)
        || !super::super::allowed_resume_executor_handoff_dispatch_state(
            executor_action,
            executor_status,
            dispatch_state,
        )
        || !super::allowed_dispatch_execution_result_status(executor_action, executor_result_status)
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
    let Some(recorded) = recorded_paused_checkpoint_resume_dispatch_result_from_result_json(
        task.clone(),
        &result_json,
        now_ts,
    ) else {
        return Ok(None);
    };
    if recorded.checkpoint_id != checkpoint_id
        || recorded.executor_state != executor_state
        || recorded.executor_action != executor_action
        || recorded.executor_status != executor_status
        || recorded.dispatch_state != dispatch_state
        || recorded.executor_result_status != executor_result_status
    {
        return Ok(None);
    }

    let result_projection_claim_expires_at = now_ts.saturating_add(lease_seconds.max(1));
    let projection_pending_reason =
        projection_pending_reason(executor_action, executor_result_status);
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object_mut() else {
        return Ok(None);
    };
    let result_projection_claim = serde_json::json!({
        "schema_version": 1,
        "owner": "worker_recovery_result_projector",
        "checkpoint_id": checkpoint_id,
        "executor_state": executor_state,
        "executor_action": executor_action,
        "executor_status": executor_status,
        "dispatch_state": dispatch_state,
        "executor_result_status": executor_result_status,
        "result_projection_state": recorded.result_projection_state.as_str(),
        "projection_pending_reason": projection_pending_reason,
        "claimed_at": now_ts,
        "expires_at": result_projection_claim_expires_at
    });
    obj.insert(
        "resume_executor_result_projection_claim".to_string(),
        result_projection_claim.clone(),
    );
    if let Some(result_obj) = obj
        .get_mut("resume_executor_dispatch_result")
        .and_then(serde_json::Value::as_object_mut)
    {
        result_obj.insert(
            "projection_claim_state".to_string(),
            serde_json::json!("claimed"),
        );
        result_obj.insert(
            "projection_claim_owner".to_string(),
            serde_json::json!("worker_recovery_result_projector"),
        );
        result_obj.insert(
            "projection_claimed_at".to_string(),
            serde_json::json!(now_ts),
        );
        result_obj.insert(
            "projection_claim_expires_at".to_string(),
            serde_json::json!(result_projection_claim_expires_at),
        );
        result_obj.insert(
            "result_projection_state".to_string(),
            serde_json::json!(recorded.result_projection_state.as_str()),
        );
        result_obj.insert(
            "projection_pending_reason".to_string(),
            serde_json::json!(projection_pending_reason),
        );
    }
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
                "result_projection_claimed_at".to_string(),
                serde_json::json!(now_ts),
            );
            target_obj.insert(
                "result_projection_claim_expires_at".to_string(),
                serde_json::json!(result_projection_claim_expires_at),
            );
            target_obj.insert(
                "result_projection_state".to_string(),
                serde_json::json!(recorded.result_projection_state.as_str()),
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

    Ok(Some(ClaimedPausedCheckpointResumeDispatchResult {
        task,
        task_id: recorded.task_id,
        checkpoint_id: recorded.checkpoint_id,
        executor_state: recorded.executor_state,
        executor_action: recorded.executor_action,
        executor_status: recorded.executor_status,
        dispatch_state: recorded.dispatch_state,
        executor_result_status: recorded.executor_result_status,
        result_projection_state: recorded.result_projection_state,
        recorded_at: recorded.recorded_at,
        result_projection_claim_expires_at,
        execution_result_payload: recorded.execution_result_payload,
        result_projection_claim,
        task_checkpoint: recorded.task_checkpoint,
    }))
}

pub(crate) fn record_claimed_paused_checkpoint_resume_dispatch_result_projection_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
    executor_result_status: &str,
    projection_payload: &Value,
    now_ts: i64,
) -> anyhow::Result<bool> {
    let task_id = task_id.trim();
    let checkpoint_id = checkpoint_id.trim();
    let executor_state = executor_state.trim();
    let executor_action = executor_action.trim();
    let executor_status = executor_status.trim();
    let dispatch_state = dispatch_state.trim();
    let executor_result_status = executor_result_status.trim();
    if task_id.is_empty()
        || checkpoint_id.is_empty()
        || executor_state.is_empty()
        || executor_action.is_empty()
        || !super::super::allowed_resume_executor_handoff_status(executor_status)
        || !super::super::allowed_resume_executor_handoff_dispatch_state(
            executor_action,
            executor_status,
            dispatch_state,
        )
        || !super::allowed_dispatch_execution_result_status(executor_action, executor_result_status)
        || !projection_payload.is_object()
        || projection_payload.get("text").is_some()
        || projection_payload.get("error_text").is_some()
    {
        return Ok(false);
    }
    let projection_checkpoint_id = projection_payload
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_executor_state = projection_payload
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_executor_action = projection_payload
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_executor_status = projection_payload
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_dispatch_state = projection_payload
        .get("dispatch_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_result_status = projection_payload
        .get("executor_result_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let result_projection_state = projection_payload
        .get("result_projection_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let expected_projection_state =
        dispatch_result_projection_state(executor_action, executor_result_status)
            .unwrap_or_default();
    if projection_checkpoint_id != checkpoint_id
        || projection_executor_state != executor_state
        || projection_executor_action != executor_action
        || projection_executor_status != executor_status
        || projection_dispatch_state != dispatch_state
        || projection_result_status != executor_result_status
        || result_projection_state != expected_projection_state
    {
        return Ok(false);
    }
    if terminal_dispatch_result_status(executor_action, executor_result_status) {
        return record_claimed_paused_checkpoint_resume_terminal_projection_internal(
            state,
            task_id,
            checkpoint_id,
            executor_state,
            executor_action,
            executor_status,
            dispatch_state,
            executor_result_status,
            expected_projection_state,
            projection_payload,
            now_ts,
        );
    }
    let Some(target_executor_state) =
        rescheduled_dispatch_result_target_executor_state(executor_action, executor_result_status)
    else {
        return Ok(false);
    };
    let next_check_after = projection_payload
        .get("next_check_after")
        .and_then(Value::as_i64)
        .or_else(|| {
            projection_payload
                .get("retry_after_seconds")
                .and_then(Value::as_i64)
                .filter(|seconds| *seconds > 0)
                .map(|seconds| now_ts.saturating_add(seconds))
        })
        .filter(|ts| *ts > now_ts);
    let Some(next_check_after) = next_check_after else {
        return Ok(false);
    };

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
    if super::super::matching_resume_executor_handoff(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
    )
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
    if super::matching_resume_executor_handoff_dispatch(
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
    if super::matching_resume_executor_dispatch_result(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
    )
    .and_then(|result| result.get("executor_result_status").and_then(Value::as_str))
    .map(str::trim)
        != Some(executor_result_status)
    {
        return Ok(false);
    }
    if !active_resume_executor_result_projection_claim(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
        executor_result_status,
        now_ts,
    ) || matching_resume_executor_result_projection(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
        executor_result_status,
    )
    .is_some()
    {
        return Ok(false);
    }

    let wait_seconds = next_check_after.saturating_sub(now_ts).max(0);
    let resume_directive = obj
        .get("resume_executor")
        .and_then(|value| value.get("resume_directive"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if executor_action == "poll_async_job" {
                "poll_async_job"
            } else {
                "run_next_planner_round"
            }
        });
    let resume_trigger = obj
        .get("resume_executor")
        .and_then(|value| value.get("resume_trigger"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("worker_recovery");

    let mut projection = projection_payload.clone();
    if let Some(projection_obj) = projection.as_object_mut() {
        projection_obj.insert("projected_at".to_string(), serde_json::json!(now_ts));
        projection_obj.insert(
            "projection_result_status".to_string(),
            serde_json::json!("rescheduled"),
        );
    }
    let mut resume_executor = serde_json::json!({
        "schema_version": 1,
        "checkpoint_id": checkpoint_id,
        "executor_state": target_executor_state,
        "resume_trigger": resume_trigger,
        "resume_directive": resume_directive,
        "next_check_after": next_check_after,
        "result_projection_state": expected_projection_state,
        "executor_result_status": executor_result_status,
        "projected_at": now_ts
    });
    if executor_action == "poll_async_job" {
        for key in [
            "job_id",
            "cancel_ref",
            "message_key",
            "poll_after_seconds",
            "expires_at",
        ] {
            if let Some(value) = projection_payload.get(key).cloned().or_else(|| {
                obj.get("resume_executor")
                    .and_then(|item| item.get(key))
                    .cloned()
            }) {
                resume_executor[key] = value;
            }
        }
    }

    obj.insert("state".to_string(), serde_json::json!("background"));
    obj.insert(
        "resume_reason".to_string(),
        serde_json::json!(executor_result_status),
    );
    obj.insert(
        "next_check_after".to_string(),
        serde_json::json!(next_check_after),
    );
    obj.insert("resume_due".to_string(), serde_json::json!(false));
    obj.insert(
        "resume_wait_seconds".to_string(),
        serde_json::json!(wait_seconds),
    );
    obj.insert("resume_executor".to_string(), resume_executor);
    obj.insert("resume_executor_result_projection".to_string(), projection);
    for key in [
        "resume_claim",
        "resume_work_item",
        "resume_executor_claim",
        "resume_execution_plan",
        "resume_executor_handoff",
        "resume_executor_handoff_claim",
        "resume_executor_handoff_dispatch",
        "resume_executor_dispatch_claim",
        "resume_executor_dispatch_result",
        "resume_executor_result_projection_claim",
    ] {
        obj.remove(key);
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

fn record_claimed_paused_checkpoint_resume_terminal_projection_internal(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
    executor_result_status: &str,
    expected_projection_state: &str,
    projection_payload: &Value,
    now_ts: i64,
) -> anyhow::Result<bool> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let task_row = db
        .query_row(
            "SELECT kind, result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            params![task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?
        .and_then(|(kind, result_json)| result_json.map(|raw| (kind, raw)));
    let Some((task_kind, raw_result_json)) = task_row else {
        return Ok(false);
    };
    let result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(obj) = lifecycle.as_object() else {
        return Ok(false);
    };
    if super::super::matching_resume_executor_handoff(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
    )
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
    if super::matching_resume_executor_handoff_dispatch(
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
    if super::matching_resume_executor_dispatch_result(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
    )
    .and_then(|result| result.get("executor_result_status").and_then(Value::as_str))
    .map(str::trim)
        != Some(executor_result_status)
    {
        return Ok(false);
    }
    if !active_resume_executor_result_projection_claim(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
        executor_result_status,
        now_ts,
    ) || matching_resume_executor_result_projection(
        obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
        executor_result_status,
    )
    .is_some()
    {
        return Ok(false);
    }

    let mut projection = projection_payload.clone();
    let Some(projection_obj) = projection.as_object_mut() else {
        return Ok(false);
    };
    projection_obj.insert("projected_at".to_string(), serde_json::json!(now_ts));
    projection_obj.insert(
        "projection_result_status".to_string(),
        serde_json::json!(terminal_projection_result_status(executor_result_status)),
    );

    let (db_status, error_text, mut terminal_result) =
        terminal_task_projection_result(executor_result_status, projection_payload)?;
    if let Some(existing_result) =
        preserved_visible_ask_result_for_terminal_projection(&task_kind, &result_json, db_status)
    {
        terminal_result = existing_result;
    } else if let Some(machine_reply_result) = ask_agent_loop_async_poll_terminal_machine_reply(
        &task_kind,
        &result_json,
        db_status,
        checkpoint_id,
        executor_action,
        executor_result_status,
        projection_payload,
        &terminal_result,
    ) {
        terminal_result = machine_reply_result;
    }
    let Some(result_obj) = terminal_result.as_object_mut() else {
        return Ok(false);
    };
    result_obj.insert(
        "task_lifecycle".to_string(),
        serde_json::json!({
            "schema_version": 1,
            "state": terminal_lifecycle_state(executor_result_status),
            "checkpoint_id": checkpoint_id,
            "resume_reason": executor_result_status,
            "terminal_projection_state": expected_projection_state,
            "terminal_executor_action": executor_action,
            "terminal_executor_status": executor_status,
            "terminal_dispatch_state": dispatch_state,
            "terminal_executor_result_status": executor_result_status,
            "terminal_projected_at": now_ts,
            "resume_executor_result_projection": projection,
        }),
    );

    let updated_result_json = terminal_result.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET status = ?2,
             result_json = ?3,
             error_text = ?4,
             updated_at = ?5
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?6",
        params![
            task_id,
            db_status,
            updated_result_json,
            error_text,
            now_ts.to_string(),
            raw_result_json
        ],
    )?;
    Ok(changed > 0)
}

fn preserved_visible_ask_result_for_terminal_projection(
    task_kind: &str,
    result_json: &Value,
    db_status: &str,
) -> Option<Value> {
    if task_kind != "ask" || db_status != "succeeded" || !result_has_visible_reply(result_json) {
        return None;
    }
    result_json.as_object().map(|_| result_json.clone())
}

fn ask_agent_loop_async_poll_terminal_machine_reply(
    task_kind: &str,
    source_result_json: &Value,
    db_status: &str,
    checkpoint_id: &str,
    executor_action: &str,
    executor_result_status: &str,
    projection_payload: &Value,
    terminal_result: &Value,
) -> Option<Value> {
    if task_kind != "ask"
        || db_status != "succeeded"
        || executor_action != "poll_async_job"
        || executor_result_status != "async_poll_completed"
        || !checkpoint_id.starts_with("agent-loop:")
        || result_has_visible_reply(terminal_result)
    {
        return None;
    }

    let mut reply = Map::new();
    reply.insert("schema_version".to_string(), serde_json::json!(1));
    reply.insert(
        "output_format".to_string(),
        serde_json::json!("machine_json"),
    );
    reply.insert("status".to_string(), serde_json::json!("succeeded"));
    reply.insert(
        "checkpoint_id".to_string(),
        serde_json::json!(checkpoint_id),
    );
    if let Some(value) = first_machine_value(
        source_result_json,
        projection_payload,
        &[
            "/task_lifecycle/poll_ref",
            "/task_checkpoint/pending_async_job/job_id",
            "/task_checkpoint/pending_async_job/cancel_ref",
        ],
        &[
            "/poll_ref",
            "/job_id",
            "/cancel_ref",
            "/final_result_json/poll_ref",
            "/final_result_json/job_id",
            "/final_result_json/cancel_ref",
        ],
    ) {
        reply.insert("poll_ref".to_string(), value);
    }
    if let Some(value) = first_machine_value(
        source_result_json,
        projection_payload,
        &[
            "/task_lifecycle/next_check_after",
            "/task_lifecycle/poll_after_seconds",
            "/task_checkpoint/pending_async_job/poll_after_seconds",
        ],
        &[
            "/next_check_after",
            "/poll_after_seconds",
            "/final_result_json/next_check_after",
            "/final_result_json/poll_after_seconds",
        ],
    ) {
        reply.insert("next_check_after".to_string(), value);
    }
    if let Some(value) = first_machine_value(
        source_result_json,
        projection_payload,
        &[
            "/task_lifecycle/async_job_message_key",
            "/task_checkpoint/pending_async_job/message_key",
        ],
        &["/message_key", "/final_result_json/message_key"],
    ) {
        reply.insert("message_key".to_string(), value);
    }
    if let Some(value) = first_machine_value(
        source_result_json,
        projection_payload,
        &["/task_id"],
        &["/task_id", "/final_result_json/task_id"],
    ) {
        reply.insert("task_id".to_string(), value);
    }
    if let Some(final_result_json) = projection_payload
        .get("final_result_json")
        .cloned()
        .filter(Value::is_object)
    {
        reply.insert("final_result_json".to_string(), final_result_json);
    }

    let machine_reply = Value::Object(reply);
    let reply_text = machine_reply.to_string();
    Some(serde_json::json!({
        "text": reply_text,
        "messages": [reply_text],
        "machine_reply": machine_reply,
    }))
}

fn first_machine_value(
    primary: &Value,
    fallback: &Value,
    primary_pointers: &[&str],
    fallback_pointers: &[&str],
) -> Option<Value> {
    machine_value_by_pointers(primary, primary_pointers)
        .or_else(|| machine_value_by_pointers(fallback, fallback_pointers))
}

fn machine_value_by_pointers(root: &Value, pointers: &[&str]) -> Option<Value> {
    pointers
        .iter()
        .find_map(|pointer| machine_value_at_pointer(root, pointer))
}

fn machine_value_at_pointer(root: &Value, pointer: &str) -> Option<Value> {
    match root.pointer(pointer)? {
        Value::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| serde_json::json!(trimmed))
        }
        Value::Number(_) | Value::Bool(_) => root.pointer(pointer).cloned(),
        _ => None,
    }
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

fn recorded_paused_checkpoint_resume_dispatch_result_from_result_json(
    task: ClaimedTask,
    result_json: &Value,
    now_ts: i64,
) -> Option<RecordedPausedCheckpointResumeDispatchResult> {
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
    let executor_state = execution_plan
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let plan_checkpoint_id = execution_plan
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if plan_checkpoint_id != checkpoint_id {
        return None;
    }
    let handoff_payload = super::super::matching_resume_executor_handoff(
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
    let dispatch_payload = super::matching_resume_executor_handoff_dispatch(
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
    let result_payload = super::matching_resume_executor_dispatch_result(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
    )?;
    if result_payload.get("text").is_some() || result_payload.get("error_text").is_some() {
        return None;
    }
    let executor_result_status = result_payload
        .get("executor_result_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let result_projection_state =
        dispatch_result_projection_state(executor_action, executor_result_status)?;
    if active_resume_executor_result_projection_claim(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
        executor_result_status,
        now_ts,
    ) {
        return None;
    }
    if matching_resume_executor_result_projection(
        lifecycle_obj,
        checkpoint_id,
        executor_state,
        executor_action,
        executor_status,
        dispatch_state,
        executor_result_status,
    )
    .is_some()
    {
        return None;
    }
    let recorded_at = result_payload
        .get("recorded_at")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Some(RecordedPausedCheckpointResumeDispatchResult {
        task_id: task.task_id.clone(),
        task,
        checkpoint_id: checkpoint_id.to_string(),
        executor_state: executor_state.to_string(),
        executor_action: executor_action.to_string(),
        executor_status: executor_status.to_string(),
        dispatch_state: dispatch_state.to_string(),
        executor_result_status: executor_result_status.to_string(),
        result_projection_state: result_projection_state.to_string(),
        recorded_at,
        execution_result_payload: result_payload.clone(),
        task_checkpoint,
    })
}

fn matching_resume_executor_result_projection<'a>(
    lifecycle_obj: &'a serde_json::Map<String, Value>,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
    executor_result_status: &str,
) -> Option<&'a Value> {
    let projection = lifecycle_obj
        .get("resume_executor_result_projection")
        .filter(|value| value.is_object())?;
    if projection.get("text").is_some() || projection.get("error_text").is_some() {
        return Some(projection);
    }
    let projection_checkpoint_id = projection
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_executor_state = projection
        .get("executor_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_executor_action = projection
        .get("executor_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_executor_status = projection
        .get("executor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_dispatch_state = projection
        .get("dispatch_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_result_status = projection
        .get("executor_result_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let projection_state = projection
        .get("result_projection_state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if projection_checkpoint_id == checkpoint_id
        && projection_executor_state == executor_state
        && projection_executor_action == executor_action
        && projection_executor_status == executor_status
        && projection_dispatch_state == dispatch_state
        && projection_result_status == executor_result_status
        && dispatch_result_projection_state(executor_action, executor_result_status)
            == Some(projection_state)
    {
        Some(projection)
    } else {
        None
    }
}

fn dispatch_result_projection_state(
    executor_action: &str,
    executor_result_status: &str,
) -> Option<&'static str> {
    match (executor_action, executor_result_status) {
        ("run_seeded_agent_loop", "seeded_loop_completed") => Some("project_seeded_loop_completed"),
        ("run_seeded_agent_loop", "seeded_loop_deferred") => Some("project_seeded_loop_deferred"),
        ("run_seeded_agent_loop", "seeded_loop_failed") => Some("project_seeded_loop_failed"),
        ("poll_async_job", "async_poll_completed") => Some("project_async_poll_completed"),
        ("poll_async_job", "async_poll_rescheduled") => Some("project_async_poll_rescheduled"),
        ("poll_async_job", "async_poll_failed") => Some("project_async_poll_failed"),
        ("poll_async_job", "async_poll_cancelled") => Some("project_async_poll_cancelled"),
        ("verify_and_finalize", "finalize_completed") => Some("project_finalize_completed"),
        ("verify_and_finalize", "finalize_failed") => Some("project_finalize_failed"),
        _ => None,
    }
}

fn rescheduled_dispatch_result_target_executor_state(
    executor_action: &str,
    executor_result_status: &str,
) -> Option<&'static str> {
    match (executor_action, executor_result_status) {
        ("run_seeded_agent_loop", "seeded_loop_deferred") => Some("ready_for_planner_resume"),
        ("poll_async_job", "async_poll_rescheduled") => Some("poll_scheduled"),
        _ => None,
    }
}

fn terminal_dispatch_result_status(executor_action: &str, executor_result_status: &str) -> bool {
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

fn projection_pending_reason(executor_action: &str, executor_result_status: &str) -> &'static str {
    if terminal_dispatch_result_status(executor_action, executor_result_status) {
        "terminal_projection_pending"
    } else {
        "result_projection_pending"
    }
}

fn terminal_projection_result_status(executor_result_status: &str) -> &'static str {
    if terminal_projection_is_cancelled(executor_result_status) {
        return "terminal_cancelled";
    }
    if terminal_projection_is_failure(executor_result_status) {
        "terminal_failed"
    } else {
        "terminal_completed"
    }
}

fn terminal_lifecycle_state(executor_result_status: &str) -> &'static str {
    if terminal_projection_is_cancelled(executor_result_status) {
        return "cancelled";
    }
    if terminal_projection_is_failure(executor_result_status) {
        "failed"
    } else {
        "succeeded"
    }
}

fn terminal_projection_is_failure(executor_result_status: &str) -> bool {
    matches!(
        executor_result_status,
        "seeded_loop_failed" | "async_poll_failed" | "finalize_failed"
    )
}

fn terminal_projection_is_cancelled(executor_result_status: &str) -> bool {
    executor_result_status == "async_poll_cancelled"
}

fn terminal_task_projection_result(
    executor_result_status: &str,
    projection_payload: &Value,
) -> anyhow::Result<(&'static str, Option<String>, Value)> {
    if terminal_projection_is_cancelled(executor_result_status) {
        let mut result = projection_payload
            .get("cancellation_result_json")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| {
                serde_json::json!({
                    "status": "cancelled",
                    "message_key": "clawd.task.cancelled",
                })
            });
        if let Some(obj) = result.as_object_mut() {
            obj.insert("status".to_string(), serde_json::json!("cancelled"));
            obj.entry("message_key".to_string())
                .or_insert_with(|| serde_json::json!("clawd.task.cancelled"));
            obj.insert(
                "terminal_reason".to_string(),
                serde_json::json!("user_cancelled"),
            );
        }
        return Ok(("canceled", Some("user_cancelled".to_string()), result));
    }

    if terminal_projection_is_failure(executor_result_status) {
        let error_text = projection_payload
            .get("error_code")
            .and_then(Value::as_str)
            .or_else(|| {
                projection_payload
                    .get("message_key")
                    .and_then(Value::as_str)
            })
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        let Some(error_text) = error_text else {
            return Ok(("failed", None, Value::Null));
        };
        let mut result = projection_payload
            .get("failure_result_json")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(|| {
                serde_json::json!({
                    "status": "error",
                    "error_code": error_text,
                })
            });
        if let Some(obj) = result.as_object_mut() {
            obj.entry("status".to_string())
                .or_insert_with(|| serde_json::json!("error"));
            if let Some(message_key) = projection_payload
                .get("message_key")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                obj.entry("message_key".to_string())
                    .or_insert_with(|| serde_json::json!(message_key));
            }
            obj.entry("error_code".to_string())
                .or_insert_with(|| serde_json::json!(error_text));
        }
        return Ok(("failed", Some(error_text), result));
    }

    let result = projection_payload
        .get("final_result_json")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or(Value::Null);
    Ok(("succeeded", None, result))
}

fn active_resume_executor_result_projection_claim(
    lifecycle_obj: &serde_json::Map<String, Value>,
    checkpoint_id: &str,
    executor_state: &str,
    executor_action: &str,
    executor_status: &str,
    dispatch_state: &str,
    executor_result_status: &str,
    now_ts: i64,
) -> bool {
    let Some(claim) = lifecycle_obj
        .get("resume_executor_result_projection_claim")
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
    let claim_result_status = claim
        .get("executor_result_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let expires_at = claim.get("expires_at").and_then(Value::as_i64).unwrap_or(0);
    claim_checkpoint_id == checkpoint_id
        && claim_executor_state == executor_state
        && claim_executor_action == executor_action
        && claim_executor_status == executor_status
        && claim_dispatch_state == dispatch_state
        && claim_result_status == executor_result_status
        && expires_at > now_ts
}

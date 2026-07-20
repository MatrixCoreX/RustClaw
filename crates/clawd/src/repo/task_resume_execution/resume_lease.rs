use rusqlite::{params, OptionalExtension};
use serde_json::{json, Map, Value};

use crate::AppState;

use super::ClaimedDispatchedPausedCheckpointResumeExecution;

pub(crate) fn merge_progress_with_active_resume_coordination(
    current_result: &Value,
    progress_result: &Value,
    now_ts: i64,
) -> Option<Value> {
    let lifecycle = crate::task_lifecycle::task_query_lifecycle_projection(
        "running",
        Some(current_result),
        None,
    );
    let lifecycle_obj = lifecycle.as_object()?;
    let checkpoint_id = active_dispatch_checkpoint_id(lifecycle_obj, current_result)?;

    let mut merged = current_result.clone();
    let merged_obj = merged.as_object_mut()?;
    if let Some(progress_messages) = progress_result
        .get("progress_messages")
        .filter(|value| value.is_array())
    {
        merged_obj.insert("progress_messages".to_string(), progress_messages.clone());
    }
    merged_obj.insert(
        "resume_execution_progress".to_string(),
        json!({
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "updated_at": now_ts,
            "payload": progress_result,
        }),
    );
    Some(merged)
}

pub(crate) fn renew_claimed_dispatched_paused_checkpoint_resume_execution_lease_internal(
    state: &AppState,
    claimed: &ClaimedDispatchedPausedCheckpointResumeExecution,
    now_ts: i64,
    lease_seconds: i64,
) -> anyhow::Result<bool> {
    if !valid_claimed_identity(claimed) {
        return Ok(false);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let task_row = db
        .query_row(
            "SELECT result_json, lease_owner, lease_expires_at, COALESCE(claim_attempt, 0)
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            params![claimed.task_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((Some(raw_result_json), lease_owner, task_lease_expires_at, active_claim_attempt)) =
        task_row
    else {
        return Ok(false);
    };
    if lease_owner.as_deref() != Some(state.worker.worker_id.as_str())
        || active_claim_attempt != claimed.task.claim_attempt
        || task_lease_expires_at <= now_ts
    {
        return Ok(false);
    }

    let mut result_json = match serde_json::from_str::<Value>(&raw_result_json) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result_json), None);
    let Some(lifecycle_obj) = lifecycle.as_object_mut() else {
        return Ok(false);
    };
    if !active_claim_chain_matches(lifecycle_obj, &result_json, claimed, state, now_ts) {
        return Ok(false);
    }

    let expires_at = now_ts.saturating_add(lease_seconds.max(1));
    renew_claim(lifecycle_obj.get_mut("resume_claim"), now_ts, expires_at);
    renew_claim(
        lifecycle_obj.get_mut("resume_executor_claim"),
        now_ts,
        expires_at,
    );
    renew_claim(
        lifecycle_obj.get_mut("resume_executor_handoff_claim"),
        now_ts,
        expires_at,
    );
    renew_claim(
        lifecycle_obj.get_mut("resume_executor_dispatch_claim"),
        now_ts,
        expires_at,
    );
    renew_embedded_claim_expiries(lifecycle_obj, now_ts, expires_at);

    result_json["task_lifecycle"] = lifecycle;
    let updated_result_json = result_json.to_string();
    let changed = db.execute(
        "UPDATE tasks
         SET result_json = ?2,
             updated_at = ?3,
             lease_expires_at = ?4
         WHERE task_id = ?1
           AND status = 'running'
           AND result_json = ?5
           AND lease_owner = ?6
           AND lease_expires_at > ?3
           AND claim_attempt = ?7",
        params![
            claimed.task_id,
            updated_result_json,
            now_ts,
            expires_at,
            raw_result_json,
            state.worker.worker_id,
            claimed.task.claim_attempt
        ],
    )?;
    Ok(changed > 0)
}

fn active_dispatch_checkpoint_id<'a>(
    lifecycle: &'a Map<String, Value>,
    current_result: &Value,
) -> Option<&'a str> {
    if lifecycle
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        != Some("running")
    {
        return None;
    }
    let checkpoint_id = lifecycle
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let stored_checkpoint =
        crate::task_lifecycle::task_checkpoint_from_result_json(current_result)?;
    if stored_checkpoint.checkpoint_id != checkpoint_id {
        return None;
    }
    let dispatch_claim = lifecycle
        .get("resume_executor_dispatch_claim")
        .filter(|value| value.is_object())?;
    if dispatch_claim.get("text").is_some() || dispatch_claim.get("error_text").is_some() {
        return None;
    }
    let dispatch_checkpoint_id = dispatch_claim
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim);
    (dispatch_checkpoint_id == Some(checkpoint_id)).then_some(checkpoint_id)
}

fn valid_claimed_identity(claimed: &ClaimedDispatchedPausedCheckpointResumeExecution) -> bool {
    !claimed.task_id.trim().is_empty()
        && !claimed.checkpoint_id.trim().is_empty()
        && !claimed.executor_state.trim().is_empty()
        && !claimed.executor_action.trim().is_empty()
        && !claimed.executor_status.trim().is_empty()
        && !claimed.dispatch_state.trim().is_empty()
}

fn active_claim_chain_matches(
    lifecycle: &Map<String, Value>,
    result_json: &Value,
    claimed: &ClaimedDispatchedPausedCheckpointResumeExecution,
    state: &AppState,
    now_ts: i64,
) -> bool {
    lifecycle
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        == Some("running")
        && lifecycle
            .get("checkpoint_id")
            .and_then(Value::as_str)
            .map(str::trim)
            == Some(claimed.checkpoint_id.as_str())
        && crate::task_lifecycle::task_checkpoint_from_result_json(result_json)
            .is_some_and(|checkpoint| checkpoint.checkpoint_id == claimed.checkpoint_id)
        && claim_matches(
            lifecycle.get("resume_claim"),
            &claimed.checkpoint_id,
            None,
            None,
            None,
            None,
            Some(state.worker.worker_id.as_str()),
            now_ts,
        )
        && claim_matches(
            lifecycle.get("resume_executor_claim"),
            &claimed.checkpoint_id,
            Some(&claimed.executor_state),
            None,
            None,
            None,
            Some("worker_recovery_executor"),
            now_ts,
        )
        && claim_matches(
            lifecycle.get("resume_executor_handoff_claim"),
            &claimed.checkpoint_id,
            Some(&claimed.executor_state),
            Some(&claimed.executor_action),
            Some(&claimed.executor_status),
            None,
            Some("worker_recovery_handoff_executor"),
            now_ts,
        )
        && claim_matches(
            lifecycle.get("resume_executor_dispatch_claim"),
            &claimed.checkpoint_id,
            Some(&claimed.executor_state),
            Some(&claimed.executor_action),
            Some(&claimed.executor_status),
            Some(&claimed.dispatch_state),
            Some("worker_recovery_dispatch_executor"),
            now_ts,
        )
}

#[allow(clippy::too_many_arguments)]
fn claim_matches(
    claim: Option<&Value>,
    checkpoint_id: &str,
    executor_state: Option<&str>,
    executor_action: Option<&str>,
    executor_status: Option<&str>,
    dispatch_state: Option<&str>,
    owner: Option<&str>,
    now_ts: i64,
) -> bool {
    let Some(claim) = claim.filter(|value| value.is_object()) else {
        return false;
    };
    if claim.get("text").is_some() || claim.get("error_text").is_some() {
        return false;
    }
    claim
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        == Some(checkpoint_id)
        && optional_token_matches(claim, "executor_state", executor_state)
        && optional_token_matches(claim, "executor_action", executor_action)
        && optional_token_matches(claim, "executor_status", executor_status)
        && optional_token_matches(claim, "dispatch_state", dispatch_state)
        && optional_token_matches(claim, "owner", owner)
        && claim.get("expires_at").and_then(Value::as_i64).unwrap_or(0) > now_ts
}

fn optional_token_matches(claim: &Value, key: &str, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| {
        claim.get(key).and_then(Value::as_str).map(str::trim) == Some(expected)
    })
}

fn renew_claim(claim: Option<&mut Value>, now_ts: i64, expires_at: i64) {
    let Some(claim) = claim.and_then(Value::as_object_mut) else {
        return;
    };
    claim.insert("renewed_at".to_string(), json!(now_ts));
    claim.insert("expires_at".to_string(), json!(expires_at));
    let renewal_count = claim
        .get("renewal_count")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .saturating_add(1);
    claim.insert("renewal_count".to_string(), json!(renewal_count));
}

fn renew_embedded_claim_expiries(lifecycle: &mut Map<String, Value>, now_ts: i64, expires_at: i64) {
    if let Some(executor) = lifecycle
        .get_mut("resume_executor")
        .and_then(Value::as_object_mut)
    {
        executor.insert("executor_claim_expires_at".to_string(), json!(expires_at));
        executor.insert("handoff_claim_expires_at".to_string(), json!(expires_at));
        executor.insert("dispatch_claim_expires_at".to_string(), json!(expires_at));
        executor.insert("lease_renewed_at".to_string(), json!(now_ts));
    }
    if let Some(handoff) = lifecycle
        .get_mut("resume_executor_handoff")
        .and_then(Value::as_object_mut)
    {
        handoff.insert("claim_expires_at".to_string(), json!(expires_at));
        handoff.insert("dispatch_claim_expires_at".to_string(), json!(expires_at));
    }
    if let Some(dispatch) = lifecycle
        .get_mut("resume_executor_handoff_dispatch")
        .and_then(Value::as_object_mut)
    {
        dispatch.insert("claim_expires_at".to_string(), json!(expires_at));
    }
    for key in ["resume_executor_claim", "resume_executor_handoff_claim"] {
        if let Some(claim) = lifecycle.get_mut(key).and_then(Value::as_object_mut) {
            claim.insert("dispatch_claim_expires_at".to_string(), json!(expires_at));
        }
    }
}

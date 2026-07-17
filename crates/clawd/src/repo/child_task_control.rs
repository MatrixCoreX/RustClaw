use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{now_ts, AppState};

const MAX_CHILD_RETRY_COUNT: u64 = 4;
const MAX_REVISED_GOAL_CHARS: usize = 8_000;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChildTaskRetryUpdate {
    pub(crate) parent_task_id: String,
    pub(crate) previous_child_task_id: String,
    pub(crate) child_task_id: String,
    pub(crate) retry_index: u64,
    pub(crate) lifecycle: Value,
}

pub(crate) fn retry_child_task_with_revised_goal(
    state: &AppState,
    parent_task_id: &str,
    child_task_id: &str,
    revised_goal: &str,
) -> anyhow::Result<Option<ChildTaskRetryUpdate>> {
    let parent_task_id =
        machine_task_id(parent_task_id).ok_or_else(|| anyhow::anyhow!("parent_task_id_invalid"))?;
    let child_task_id =
        machine_task_id(child_task_id).ok_or_else(|| anyhow::anyhow!("child_task_id_invalid"))?;
    let revised_goal = bounded_revised_goal(revised_goal)?;
    let now = now_ts();
    let mut db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("db_pool:{error}"))?;
    let tx = db.transaction()?;

    let child_row = tx
        .query_row(
            "SELECT user_id, chat_id, user_key, channel, external_user_id,
                    external_chat_id, kind, payload_json, status
             FROM tasks
             WHERE task_id = ?1
             LIMIT 1",
            params![child_task_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()?;
    let Some((
        user_id,
        chat_id,
        user_key,
        channel,
        external_user_id,
        external_chat_id,
        kind,
        raw_payload,
        status,
    )) = child_row
    else {
        return Ok(None);
    };
    if !matches!(status.as_str(), "failed" | "timeout" | "canceled") {
        anyhow::bail!("child_task_not_retryable");
    }
    let mut payload = serde_json::from_str::<Value>(&raw_payload)
        .map_err(|_| anyhow::anyhow!("child_task_payload_invalid"))?;
    if !super::child_tasks::is_child_subagent_payload(&payload) {
        anyhow::bail!("task_not_subagent_child");
    }
    if payload
        .pointer("/child_task_contract/parent_task_id")
        .and_then(Value::as_str)
        != Some(parent_task_id)
    {
        anyhow::bail!("child_parent_mismatch");
    }

    let parent_row = tx
        .query_row(
            "SELECT user_id, chat_id, status, result_json
             FROM tasks
             WHERE task_id = ?1
             LIMIT 1",
            params![parent_task_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((parent_user_id, parent_chat_id, parent_status, raw_parent_result)) = parent_row
    else {
        anyhow::bail!("parent_task_missing");
    };
    if parent_user_id != user_id || parent_chat_id != chat_id {
        anyhow::bail!("child_parent_actor_mismatch");
    }
    if !matches!(parent_status.as_str(), "queued" | "running") {
        anyhow::bail!("parent_task_not_active");
    }

    let retry_index = payload
        .pointer("/child_retry/retry_index")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .saturating_add(1);
    if retry_index > MAX_CHILD_RETRY_COUNT {
        anyhow::bail!("child_retry_limit_exceeded");
    }
    let new_child_task_id = Uuid::new_v4().to_string();
    update_retry_payload(
        &mut payload,
        parent_task_id,
        child_task_id,
        &new_child_task_id,
        revised_goal,
        retry_index,
    )?;
    let contract = payload
        .get("child_task_contract")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("child_task_contract_missing"))?;
    let lifecycle = queued_retry_lifecycle(
        parent_task_id,
        child_task_id,
        &new_child_task_id,
        retry_index,
    );
    let result_json = json!({
        "schema_version": crate::child_task_contract::CHILD_TASK_SCHEMA_VERSION,
        "source": "child_task_retry",
        "message_key": "clawd.child_task.retry_queued",
        "child_task": contract,
        "child_retry": {
            "retry_index": retry_index,
            "previous_child_task_id": child_task_id,
            "goal_updated": true,
        },
        "task_lifecycle": lifecycle,
    });
    tx.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, external_user_id,
            external_chat_id, message_id, kind, payload_json, status,
            result_json, error_text, created_at, updated_at
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, 'queued',
                 ?10, NULL, ?11, ?11)",
        params![
            new_child_task_id,
            user_id,
            chat_id,
            user_key,
            channel,
            external_user_id,
            external_chat_id,
            kind,
            payload.to_string(),
            result_json.to_string(),
            now,
        ],
    )?;

    let mut parent_result = raw_parent_result
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    record_parent_retry(
        &mut parent_result,
        child_task_id,
        &new_child_task_id,
        retry_index,
    )?;
    let updated = tx.execute(
        "UPDATE tasks
         SET result_json = ?2, updated_at = ?3
         WHERE task_id = ?1
           AND status IN ('queued', 'running')",
        params![parent_task_id, parent_result.to_string(), now],
    )?;
    if updated != 1 {
        anyhow::bail!("parent_task_retry_update_conflict");
    }
    tx.commit()?;
    drop(db);
    let _ = super::child_tasks::refresh_parent_child_task_merge(state, parent_task_id)?;

    Ok(Some(ChildTaskRetryUpdate {
        parent_task_id: parent_task_id.to_string(),
        previous_child_task_id: child_task_id.to_string(),
        child_task_id: new_child_task_id,
        retry_index,
        lifecycle,
    }))
}

fn bounded_revised_goal(value: &str) -> anyhow::Result<&str> {
    let value = value.trim();
    let char_count = value.chars().count();
    if char_count == 0 {
        anyhow::bail!("revised_goal_required");
    }
    if char_count > MAX_REVISED_GOAL_CHARS {
        anyhow::bail!("revised_goal_too_large");
    }
    Ok(value)
}

fn update_retry_payload(
    payload: &mut Value,
    parent_task_id: &str,
    previous_child_task_id: &str,
    child_task_id: &str,
    revised_goal: &str,
    retry_index: u64,
) -> anyhow::Result<()> {
    let object = payload
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("child_task_payload_invalid"))?;
    object.insert("text".to_string(), json!(revised_goal));
    object.insert("parent_task_id".to_string(), json!(parent_task_id));
    object.insert("child_task_id".to_string(), json!(child_task_id));
    object.insert(
        "child_retry".to_string(),
        json!({
            "retry_index": retry_index,
            "previous_child_task_id": previous_child_task_id,
            "retry_reason": "parent_revised_goal",
        }),
    );
    let contract = object
        .get_mut("child_task_contract")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("child_task_contract_missing"))?;
    contract.insert("parent_task_id".to_string(), json!(parent_task_id));
    contract.insert("child_task_id".to_string(), json!(child_task_id));
    let scope = contract
        .get_mut("scope")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("child_task_scope_missing"))?;
    scope.insert("objective".to_string(), json!(revised_goal));
    Ok(())
}

fn record_parent_retry(
    parent_result: &mut Value,
    previous_child_task_id: &str,
    child_task_id: &str,
    retry_index: u64,
) -> anyhow::Result<()> {
    let object = parent_result
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("parent_result_invalid"))?;
    append_unique_string(object, "child_task_ids", child_task_id);
    append_unique_string(object, "superseded_child_task_ids", previous_child_task_id);
    let retries = object
        .entry("child_task_retries".to_string())
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("child_task_retries_invalid"))?;
    if retries.len() >= crate::child_task_contract::DEFAULT_MAX_CHILDREN_PER_PARENT {
        anyhow::bail!("child_retry_history_limit_exceeded");
    }
    retries.push(json!({
        "previous_child_task_id": previous_child_task_id,
        "child_task_id": child_task_id,
        "retry_index": retry_index,
        "reason_code": "parent_revised_goal",
    }));
    Ok(())
}

fn append_unique_string(object: &mut serde_json::Map<String, Value>, key: &str, value: &str) {
    let values = object.entry(key.to_string()).or_insert_with(|| json!([]));
    if !values.is_array() {
        *values = json!([]);
    }
    let values = values.as_array_mut().expect("normalized array");
    if !values.iter().any(|item| item.as_str() == Some(value)) {
        values.push(json!(value));
    }
}

fn queued_retry_lifecycle(
    parent_task_id: &str,
    previous_child_task_id: &str,
    child_task_id: &str,
    retry_index: u64,
) -> Value {
    json!({
        "schema_version": crate::child_task_contract::CHILD_TASK_SCHEMA_VERSION,
        "state": "queued",
        "state_source": "child_task_retry",
        "parent_task_id": parent_task_id,
        "child_task_id": child_task_id,
        "previous_child_task_id": previous_child_task_id,
        "retry_index": retry_index,
        "can_poll": true,
        "can_cancel": true,
        "can_pause": false,
        "can_steer": false,
        "can_retry": false,
    })
}

fn machine_task_id(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 160
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/'))
    {
        return None;
    }
    Some(value)
}

#[cfg(test)]
#[path = "child_task_control_tests.rs"]
mod tests;

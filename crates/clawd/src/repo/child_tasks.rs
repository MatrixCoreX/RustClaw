#![allow(dead_code)]

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::{
    child_task_contract::{child_scheduler_decision, ChildTaskSpec, CHILD_TASK_SCHEMA_VERSION},
    now_ts, AppState,
};

#[derive(Debug, Clone)]
pub(crate) struct ChildTaskParentContext {
    pub(crate) parent_task_id: String,
    pub(crate) user_id: i64,
    pub(crate) chat_id: i64,
    pub(crate) user_key: Option<String>,
    pub(crate) channel: String,
    pub(crate) external_user_id: Option<String>,
    pub(crate) external_chat_id: Option<String>,
}

pub(crate) fn enqueue_child_task_specs(
    state: &AppState,
    parent: &ChildTaskParentContext,
    specs: &[ChildTaskSpec],
    max_parallel: usize,
    recursion_depth: usize,
) -> anyhow::Result<Value> {
    let scheduler = child_scheduler_decision(specs.len(), max_parallel, recursion_depth);
    let scheduled_count = scheduler
        .get("scheduled_child_count")
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize;
    if scheduled_count == 0 {
        return Ok(json!({
            "schema_version": CHILD_TASK_SCHEMA_VERSION,
            "parent_task_id": parent.parent_task_id,
            "status": "not_scheduled",
            "queued_child_count": 0,
            "child_task_ids": [],
            "scheduler": scheduler,
        }));
    }

    let now = now_ts();
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let mut queued_child_ids = Vec::new();
    for spec in specs.iter().take(scheduled_count) {
        if spec.parent_task_id != parent.parent_task_id {
            anyhow::bail!("child_parent_mismatch");
        }
        let payload = child_task_payload(spec)?;
        let result_json = queued_child_task_result(spec);
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, channel, external_user_id,
                external_chat_id, message_id, kind, payload_json, status,
                result_json, error_text, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 'ask', ?8, 'queued', ?9, NULL, ?10, ?10)",
            params![
                spec.child_task_id,
                parent.user_id,
                parent.chat_id,
                parent.user_key,
                parent.channel,
                parent.external_user_id,
                parent.external_chat_id,
                payload.to_string(),
                result_json.to_string(),
                now
            ],
        )?;
        queued_child_ids.push(spec.child_task_id.clone());
    }
    append_parent_child_enqueue_progress(&db, parent, &queued_child_ids, &scheduler, &now)?;
    Ok(json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "parent_task_id": parent.parent_task_id,
        "status": "scheduled",
        "queued_child_count": queued_child_ids.len(),
        "child_task_ids": queued_child_ids,
        "scheduler": scheduler,
    }))
}

fn child_task_payload(spec: &ChildTaskSpec) -> anyhow::Result<Value> {
    let objective =
        child_task_objective(spec).ok_or_else(|| anyhow::anyhow!("child_objective_missing"))?;
    Ok(json!({
        "text": objective,
        "task_role": "subagent_child",
        "parent_task_id": spec.parent_task_id,
        "child_task_id": spec.child_task_id,
        "child_task_contract": spec.to_json(),
        "child_execution": {
            "schema_version": CHILD_TASK_SCHEMA_VERSION,
            "entrypoint": "agent_loop",
            "permission_profile": spec.permission_profile.as_str(),
            "required": spec.required,
            "merge_policy": spec.merge_policy.as_str(),
        },
    }))
}

fn child_task_objective(spec: &ChildTaskSpec) -> Option<&str> {
    spec.scope
        .get("objective")
        .and_then(Value::as_str)
        .or_else(|| {
            spec.result_contract
                .get("objective")
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn queued_child_task_result(spec: &ChildTaskSpec) -> Value {
    json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "source": "child_task_enqueue",
        "message_key": "clawd.child_task.queued",
        "child_task": spec.to_json(),
        "task_lifecycle": {
            "schema_version": CHILD_TASK_SCHEMA_VERSION,
            "state": "queued",
            "state_source": "child_task_enqueue",
            "parent_task_id": spec.parent_task_id,
            "child_task_id": spec.child_task_id,
            "role": spec.role,
            "permission_profile": spec.permission_profile.as_str(),
            "required": spec.required,
            "can_poll": true,
            "can_cancel": true,
        },
    })
}

fn append_parent_child_enqueue_progress(
    db: &rusqlite::Connection,
    parent: &ChildTaskParentContext,
    queued_child_ids: &[String],
    scheduler: &Value,
    now: &str,
) -> anyhow::Result<()> {
    let raw_result_json: Option<String> = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![parent.parent_task_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let mut result_json = raw_result_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| json!({}));
    if !result_json.is_object() {
        result_json = json!({});
    }
    let obj = result_json
        .as_object_mut()
        .expect("object after normalization");
    let mut child_task_ids = obj
        .get("child_task_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for child_id in queued_child_ids {
        if !child_task_ids
            .iter()
            .any(|value| value.as_str() == Some(child_id.as_str()))
        {
            child_task_ids.push(Value::String(child_id.clone()));
        }
    }
    obj.insert("child_task_ids".to_string(), Value::Array(child_task_ids));
    obj.insert(
        "child_task_enqueue".to_string(),
        json!({
            "schema_version": CHILD_TASK_SCHEMA_VERSION,
            "parent_task_id": parent.parent_task_id,
            "queued_child_count": queued_child_ids.len(),
            "child_task_ids": queued_child_ids,
            "scheduler": scheduler,
        }),
    );
    db.execute(
        "UPDATE tasks
         SET result_json = ?2, updated_at = ?3
         WHERE task_id = ?1 AND status IN ('queued', 'running')",
        params![parent.parent_task_id, result_json.to_string(), now],
    )?;
    Ok(())
}

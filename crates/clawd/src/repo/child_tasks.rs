#![allow(dead_code)]

use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use crate::{
    child_task_contract::{
        child_scheduler_decision, merge_child_task_results, ChildTaskSpec,
        CHILD_TASK_SCHEMA_VERSION,
    },
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

pub(crate) fn is_child_subagent_payload(payload: &Value) -> bool {
    payload.get("task_role").and_then(Value::as_str) == Some("subagent_child")
        && payload
            .get("child_task_contract")
            .is_some_and(Value::is_object)
}

pub(crate) fn record_child_task_terminal_projection(
    state: &AppState,
    task_id: &str,
    payload: &Value,
) -> anyhow::Result<bool> {
    if !is_child_subagent_payload(payload) {
        return Ok(false);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    let row = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((status, raw_result_json)) = row else {
        return Ok(false);
    };
    if !matches!(
        status.as_str(),
        "succeeded" | "failed" | "timeout" | "canceled"
    ) {
        return Ok(false);
    }
    let mut result_json = raw_result_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| json!({}));
    if !result_json.is_object() {
        result_json = json!({
            "observed_result_json": result_json,
        });
    }
    let child_result = child_task_result_projection(&status, payload);
    let lifecycle = child_terminal_lifecycle_projection(&status, payload);
    let Some(obj) = result_json.as_object_mut() else {
        return Ok(false);
    };
    obj.insert("child_task_result".to_string(), child_result.clone());
    obj.insert("task_lifecycle".to_string(), lifecycle);
    obj.insert(
        "child_task_id".to_string(),
        child_result["child_task_id"].clone(),
    );
    obj.insert(
        "parent_task_id".to_string(),
        child_result["parent_task_id"].clone(),
    );
    obj.insert("required".to_string(), child_result["required"].clone());
    obj.insert("status".to_string(), child_result["status"].clone());
    db.execute(
        "UPDATE tasks
         SET result_json = ?2, updated_at = ?3
         WHERE task_id = ?1 AND status = ?4",
        params![task_id, result_json.to_string(), now_ts(), status],
    )?;
    if let Some(parent_task_id) = child_result
        .get("parent_task_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        refresh_parent_child_task_merge_from_db(&db, parent_task_id)?;
    }
    Ok(true)
}

pub(crate) fn refresh_parent_child_task_merge(
    state: &AppState,
    parent_task_id: &str,
) -> anyhow::Result<Option<Value>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    refresh_parent_child_task_merge_from_db(&db, parent_task_id)
}

fn refresh_parent_child_task_merge_from_db(
    db: &rusqlite::Connection,
    parent_task_id: &str,
) -> anyhow::Result<Option<Value>> {
    let parent_row = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![parent_task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((parent_status, raw_parent_result)) = parent_row else {
        return Ok(None);
    };
    if !matches!(parent_status.as_str(), "queued" | "running") {
        return Ok(None);
    }
    let mut parent_result = raw_parent_result
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| json!({}));
    if !parent_result.is_object() {
        parent_result = json!({});
    }
    let child_task_ids = parent_child_task_ids(&parent_result);
    if child_task_ids.is_empty() {
        return Ok(None);
    }

    let mut child_results = Vec::new();
    let mut pending_child_ids = Vec::new();
    let mut missing_child_ids = Vec::new();
    for child_task_id in &child_task_ids {
        let child_row = db
            .query_row(
                "SELECT status, result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
                params![child_task_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let Some((child_status, raw_child_result)) = child_row else {
            missing_child_ids.push(child_task_id.clone());
            continue;
        };
        if !matches!(
            child_status.as_str(),
            "succeeded" | "failed" | "timeout" | "canceled"
        ) {
            pending_child_ids.push(child_task_id.clone());
            continue;
        }
        let Some(child_result) = raw_child_result
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|value| value.get("child_task_result").cloned())
            .filter(Value::is_object)
        else {
            pending_child_ids.push(child_task_id.clone());
            continue;
        };
        child_results.push(child_result);
    }

    let merge = merge_child_task_results(parent_task_id, &child_results);
    let pending_count = pending_child_ids.len();
    let missing_count = missing_child_ids.len();
    let continuation_status = if pending_count > 0 || missing_count > 0 {
        "waiting"
    } else if merge
        .get("parent_can_continue")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "ready"
    } else {
        "blocked"
    };
    let reason_code = match continuation_status {
        "waiting" => "child_tasks_pending",
        "ready" => "child_tasks_merged",
        _ => "required_child_failed",
    };
    let projection = json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "source": "child_task_parent_merge",
        "parent_task_id": parent_task_id,
        "child_task_ids": child_task_ids,
        "terminal_child_count": child_results.len(),
        "pending_child_count": pending_count,
        "missing_child_count": missing_count,
        "pending_child_ids": pending_child_ids,
        "missing_child_ids": missing_child_ids,
        "merge": merge,
        "parent_continuation": {
            "status": continuation_status,
            "reason_code": reason_code,
            "can_continue": continuation_status == "ready",
        },
    });
    let obj = parent_result
        .as_object_mut()
        .expect("object after normalization");
    obj.insert("child_task_merge".to_string(), projection.clone());
    db.execute(
        "UPDATE tasks
         SET result_json = ?2, updated_at = ?3
         WHERE task_id = ?1 AND status IN ('queued', 'running')",
        params![parent_task_id, parent_result.to_string(), now_ts()],
    )?;
    Ok(Some(projection))
}

fn parent_child_task_ids(parent_result: &Value) -> Vec<String> {
    let mut child_task_ids = Vec::new();
    append_child_task_id_array(parent_result.get("child_task_ids"), &mut child_task_ids);
    append_child_task_id_array(
        parent_result
            .get("child_task_enqueue")
            .and_then(|value| value.get("child_task_ids")),
        &mut child_task_ids,
    );
    child_task_ids
}

fn append_child_task_id_array(value: Option<&Value>, output: &mut Vec<String>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    for item in items
        .iter()
        .take(crate::child_task_contract::DEFAULT_MAX_CHILDREN_PER_PARENT)
    {
        let Some(task_id) = item.as_str().and_then(machine_task_id) else {
            continue;
        };
        if !output.iter().any(|existing| existing == &task_id) {
            output.push(task_id);
        }
    }
}

fn machine_task_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 160 {
        return None;
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn child_task_result_projection(status: &str, payload: &Value) -> Value {
    let contract = payload.get("child_task_contract").unwrap_or(&Value::Null);
    let child_task_id = contract
        .get("child_task_id")
        .and_then(Value::as_str)
        .or_else(|| payload.get("child_task_id").and_then(Value::as_str))
        .unwrap_or_default();
    let parent_task_id = contract
        .get("parent_task_id")
        .and_then(Value::as_str)
        .or_else(|| payload.get("parent_task_id").and_then(Value::as_str))
        .unwrap_or_default();
    let role = contract
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let required = contract
        .get("required")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let status = child_terminal_status(status);
    json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "parent_task_id": parent_task_id,
        "child_task_id": child_task_id,
        "role": role,
        "required": required,
        "status": status,
        "permission_profile": contract
            .get("permission_profile")
            .and_then(Value::as_str)
            .unwrap_or("read_only"),
        "merge_policy": contract
            .get("merge_policy")
            .and_then(Value::as_str)
            .unwrap_or("structured_findings"),
        "error_code": if status == "succeeded" {
            Value::Null
        } else {
            json!("child_task_terminal_not_succeeded")
        },
        "message_key": if status == "succeeded" {
            "clawd.child_task.succeeded"
        } else {
            "clawd.child_task.not_succeeded"
        },
        "evidence_refs": [format!("task:{child_task_id}:result_json")],
        "finding_refs": if status == "succeeded" {
            json!([format!("child_task:{child_task_id}:structured_result")])
        } else {
            json!([])
        },
    })
}

fn child_terminal_lifecycle_projection(status: &str, payload: &Value) -> Value {
    let contract = payload.get("child_task_contract").unwrap_or(&Value::Null);
    json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "state": child_lifecycle_state(status),
        "state_source": "child_task_terminal_projection",
        "parent_task_id": contract
            .get("parent_task_id")
            .and_then(Value::as_str)
            .or_else(|| payload.get("parent_task_id").and_then(Value::as_str))
            .unwrap_or_default(),
        "child_task_id": contract
            .get("child_task_id")
            .and_then(Value::as_str)
            .or_else(|| payload.get("child_task_id").and_then(Value::as_str))
            .unwrap_or_default(),
        "role": contract
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "required": contract
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "can_poll": false,
        "can_cancel": false,
    })
}

fn child_terminal_status(status: &str) -> &'static str {
    match status {
        "succeeded" => "succeeded",
        "canceled" => "cancelled",
        "timeout" => "failed",
        "failed" => "failed",
        _ => "unknown",
    }
}

fn child_lifecycle_state(status: &str) -> &'static str {
    match status {
        "succeeded" => "succeeded",
        "canceled" => "cancelled",
        "timeout" | "failed" => "failed",
        _ => "unknown",
    }
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

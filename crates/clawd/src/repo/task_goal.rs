use rusqlite::OptionalExtension;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::{now_ts, AppState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskGoalControlOperation {
    Edit,
    Clear,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskGoalControlUpdate {
    pub(crate) task_id: String,
    pub(crate) operation: String,
    pub(crate) goal: Option<Value>,
    pub(crate) payload_json: Value,
}

impl TaskGoalControlOperation {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "edit" => Some(Self::Edit),
            "clear" => Some(Self::Clear),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Clear => "clear",
        }
    }
}

pub(crate) fn update_task_goal_payload(
    state: &AppState,
    task_id: &str,
    operation: TaskGoalControlOperation,
    goal_patch: Option<Value>,
) -> anyhow::Result<Option<TaskGoalControlUpdate>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db_pool_failed={e}"))?;
    let Some(raw_payload) = db
        .query_row(
            "SELECT payload_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            rusqlite::params![task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };

    let mut payload = serde_json::from_str::<Value>(&raw_payload)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let Some(payload_obj) = payload.as_object_mut() else {
        return Ok(None);
    };

    match operation {
        TaskGoalControlOperation::Edit => {
            let patch = normalize_goal_patch(goal_patch.as_ref())?;
            let existing = payload_obj
                .get("goal")
                .or_else(|| payload_obj.get("goal_spec"))
                .or_else(|| payload_obj.get("task_goal"))
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({ "schema_version": 1, "goal_status": "created" }));
            let mut goal_obj = existing
                .as_object()
                .cloned()
                .unwrap_or_else(serde_json::Map::new);
            for (key, value) in patch {
                goal_obj.insert(key, value);
            }
            goal_obj
                .entry("schema_version".to_string())
                .or_insert_with(|| json!(1));
            payload_obj.remove("goal_spec");
            payload_obj.remove("task_goal");
            payload_obj.insert("goal".to_string(), Value::Object(goal_obj));
        }
        TaskGoalControlOperation::Clear => {
            payload_obj.remove("goal");
            payload_obj.remove("goal_spec");
            payload_obj.remove("task_goal");
        }
    }

    let updated_at = now_ts();
    let affected = db.execute(
        "UPDATE tasks
         SET payload_json = ?2,
             updated_at = ?3
         WHERE task_id = ?1",
        rusqlite::params![task_id, payload.to_string(), updated_at],
    )?;
    if affected == 0 {
        return Ok(None);
    }
    let goal = payload.get("goal").cloned().filter(Value::is_object);
    Ok(Some(TaskGoalControlUpdate {
        task_id: task_id.to_string(),
        operation: operation.as_str().to_string(),
        goal,
        payload_json: payload,
    }))
}

pub(crate) fn task_goal_projection(
    task_id: Uuid,
    payload_json: &str,
    result_json: Option<&Value>,
    lifecycle: &Value,
) -> Option<Value> {
    let payload = serde_json::from_str::<Value>(payload_json).ok();
    let payload_goal = payload.as_ref().and_then(payload_goal_spec);
    let result_goal = result_json.and_then(result_goal_spec);
    if payload_goal.is_none() && result_goal.is_none() {
        return None;
    }

    let mut projected = Map::new();
    projected.insert("schema_version".to_string(), json!(1));
    projected.insert("task_id".to_string(), json!(task_id.to_string()));
    projected.insert("goal_id".to_string(), json!(format!("task:{task_id}")));

    merge_goal_fields(&mut projected, payload_goal);
    merge_goal_fields(&mut projected, result_goal);
    merge_goal_progress_fields(&mut projected, result_goal);

    if !projected.contains_key("goal_status") {
        if let Some(status) =
            explicit_goal_status(payload_goal).or_else(|| explicit_goal_status(result_goal))
        {
            projected.insert("goal_status".to_string(), json!(status));
            projected.insert("goal_status_source".to_string(), json!("goal"));
        } else if let Some(status) = lifecycle_goal_status(lifecycle) {
            projected.insert("goal_status".to_string(), json!(status));
            projected.insert("goal_status_source".to_string(), json!("lifecycle"));
        }
    }

    Some(Value::Object(projected))
}

fn payload_goal_spec(payload: &Value) -> Option<&Value> {
    payload
        .get("goal")
        .or_else(|| payload.get("goal_spec"))
        .or_else(|| payload.get("task_goal"))
        .filter(|value| value.is_object())
}

fn result_goal_spec(result: &Value) -> Option<&Value> {
    result
        .get("goal")
        .or_else(|| result.get("goal_spec"))
        .or_else(|| result.get("task_goal"))
        .or_else(|| result.pointer("/task_journal/summary/goal"))
        .or_else(|| result.pointer("/task_journal/summary/task_goal"))
        .or_else(|| result.pointer("/task_journal/summary/task_outcome"))
        .filter(|value| value.is_object())
}

fn merge_goal_fields(projected: &mut Map<String, Value>, goal: Option<&Value>) {
    let Some(goal) = goal else {
        return;
    };
    for key in [
        "goal_id",
        "objective",
        "constraints",
        "done_conditions",
        "verification_commands",
        "allowed_files_or_scopes",
        "forbidden_actions",
    ] {
        copy_non_empty(projected, goal, key);
    }
    if !projected.contains_key("verification_commands") {
        if let Some(command) = goal
            .pointer("/verification/command")
            .and_then(Value::as_str)
        {
            let command = command.trim();
            if !command.is_empty() {
                projected.insert("verification_commands".to_string(), json!([command]));
            }
        }
    }
}

fn merge_goal_progress_fields(projected: &mut Map<String, Value>, goal: Option<&Value>) {
    let Some(goal) = goal else {
        return;
    };
    for key in [
        "goal_status",
        "current_progress",
        "remaining_work",
        "success_evidence_refs",
        "blocking_reason_code",
        "last_checkpoint_id",
        "last_successful_evidence_ref",
    ] {
        copy_non_empty(projected, goal, key);
    }
}

fn copy_non_empty(projected: &mut Map<String, Value>, source: &Value, key: &str) {
    let Some(value) = source.get(key) else {
        return;
    };
    if value_is_empty(value) {
        return;
    }
    projected.insert(key.to_string(), value.clone());
}

fn value_is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(value) => value.trim().is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(values) => values.is_empty(),
        Value::Bool(_) | Value::Number(_) => false,
    }
}

fn explicit_goal_status(goal: Option<&Value>) -> Option<&str> {
    goal.and_then(|goal| goal.get("goal_status").or_else(|| goal.get("state")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn lifecycle_goal_status(lifecycle: &Value) -> Option<&'static str> {
    let state = lifecycle
        .get("execution_state")
        .or_else(|| lifecycle.get("state"))
        .or_else(|| lifecycle.get("db_status"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    match state {
        "completed" | "succeeded" | "done" => Some("completed"),
        "blocked" | "failed" | "timeout" => Some("blocked"),
        "cancelled" | "canceled" => Some("cancelled"),
        "background" | "waiting" => Some("background"),
        "needs_user" | "needs_confirmation" => Some("waiting_user"),
        "queued" | "running" => Some("in_progress"),
        _ => None,
    }
}

fn normalize_goal_patch(goal_patch: Option<&Value>) -> anyhow::Result<Map<String, Value>> {
    let Some(goal_patch) = goal_patch else {
        return Err(anyhow::anyhow!("goal_patch_required"));
    };
    let Some(source) = goal_patch.as_object() else {
        return Err(anyhow::anyhow!("goal_patch_must_be_object"));
    };
    let mut normalized = Map::new();
    for key in [
        "goal_id",
        "objective",
        "constraints",
        "done_conditions",
        "verification_commands",
        "allowed_files_or_scopes",
        "forbidden_actions",
        "goal_status",
    ] {
        if let Some(value) = source.get(key) {
            if !value_is_empty(value) {
                normalized.insert(key.to_string(), value.clone());
            }
        }
    }
    if normalized.is_empty() {
        return Err(anyhow::anyhow!("goal_patch_empty"));
    }
    Ok(normalized)
}

#[cfg(test)]
#[path = "task_goal_tests.rs"]
mod tests;

use serde_json::{json, Map, Value};
use uuid::Uuid;

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

#[cfg(test)]
#[path = "task_goal_tests.rs"]
mod tests;

use serde_json::{json, Map, Value};

use crate::repo::task_plan::TASK_PLAN_SOURCE;
use crate::{AppState, ClaimedTask};

pub(super) fn execute_task_plan(
    state: &AppState,
    task: &ClaimedTask,
    args: &Map<String, Value>,
) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let result = match action.as_str() {
        "set_plan" => {
            ensure_only_keys(args, &["action", "plan_revision", "steps"])?;
            let expected_revision = optional_revision(args, true)?;
            let steps = parse_required_array(args, "steps")?;
            crate::repo::set_task_plan(state, &task.task_id, expected_revision, steps)
        }
        "update_steps" => {
            ensure_only_keys(args, &["action", "plan_revision", "updates"])?;
            let expected_revision = optional_revision(args, false)?;
            let updates = parse_required_array(args, "updates")?;
            crate::repo::update_task_plan_steps(state, &task.task_id, expected_revision, updates)
        }
        "read_plan" => {
            ensure_only_keys(args, &["action"])?;
            crate::repo::read_task_plan(state, &task.task_id, "read_plan")
        }
        _ => {
            return Err(task_plan_error(
                "task_plan_invalid_action",
                json!({"action": action}),
            ));
        }
    }
    .map_err(|error| {
        super::builtin_error(
            TASK_PLAN_SOURCE,
            error.error_code,
            error.error_code,
            None,
            None,
            Some(error.machine_extra()),
        )
    })?;

    if matches!(action.as_str(), "set_plan" | "update_steps") {
        crate::task_event_transport::publish_claimed_event(
            state,
            task,
            "task_plan_updated",
            json!({
                "schema_version": 1,
                "source": TASK_PLAN_SOURCE,
                "data_only": true,
                "render_owner": "ui_cli_channel_projection",
                "plan_revision": result.get("plan_revision"),
                "steps": result.get("steps"),
                "checkpoint": result.get("checkpoint"),
            }),
        )
        .map_err(|error| {
            task_plan_error(
                "task_plan_event_publish_failed",
                json!({
                    "retryable": true,
                    "plan_revision": result.get("plan_revision"),
                    "detail": error.to_string(),
                }),
            )
        })?;
    }

    serde_json::to_string(&result).map_err(|error| {
        task_plan_error(
            "task_plan_response_encode_failed",
            json!({"detail": error.to_string()}),
        )
    })
}

fn ensure_only_keys(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(unexpected) = args
        .keys()
        .find(|key| !allowed.iter().any(|allowed| key.as_str() == *allowed))
    {
        return Err(task_plan_error(
            "task_plan_unexpected_argument",
            json!({"argument": unexpected}),
        ));
    }
    Ok(())
}

fn optional_revision(args: &Map<String, Value>, allow_default_zero: bool) -> Result<u64, String> {
    match args.get("plan_revision") {
        Some(value) => value.as_u64().ok_or_else(|| {
            task_plan_error(
                "task_plan_revision_invalid",
                json!({"plan_revision": value}),
            )
        }),
        None if allow_default_zero => Ok(0),
        None => Err(task_plan_error("task_plan_revision_required", Value::Null)),
    }
}

fn parse_required_array<T>(args: &Map<String, Value>, key: &str) -> Result<Vec<T>, String>
where
    T: serde::de::DeserializeOwned,
{
    let Some(value) = args.get(key) else {
        return Err(task_plan_error(
            "task_plan_argument_required",
            json!({"argument": key}),
        ));
    };
    serde_json::from_value::<Vec<T>>(value.clone()).map_err(|error| {
        task_plan_error(
            "task_plan_argument_invalid",
            json!({"argument": key, "detail": error.to_string()}),
        )
    })
}

fn task_plan_error(error_code: &str, extra: Value) -> String {
    super::builtin_error(
        TASK_PLAN_SOURCE,
        error_code,
        error_code,
        None,
        None,
        Some(json!({
            "schema_version": 1,
            "source": TASK_PLAN_SOURCE,
            "status": "error",
            "error_code": error_code,
            "message_key": format!("clawd.task_plan.{error_code}"),
            "retryable": extra.get("retryable").and_then(Value::as_bool).unwrap_or(false),
            "details": extra,
        })),
    )
}

#[cfg(test)]
#[path = "builtin_task_plan_tests.rs"]
mod tests;

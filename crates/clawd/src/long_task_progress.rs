use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub(crate) const OPERATION_PROGRESS_SCHEMA_VERSION: u64 = 1;

pub(crate) fn operation_progress_from_lifecycle(
    lifecycle: &Value,
    heartbeat_fallback: Option<i64>,
) -> Value {
    let state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("running")
        .trim();
    let phase_key = lifecycle
        .get("phase_key")
        .or_else(|| lifecycle.get("stage"))
        .and_then(Value::as_str)
        .filter(|value| machine_token(value))
        .unwrap_or(state);
    let terminal = matches!(
        state,
        "succeeded" | "completed" | "failed" | "cancelled" | "canceled" | "timeout"
    );
    let completed_units = lifecycle
        .get("completed_units")
        .and_then(Value::as_u64)
        .unwrap_or(u64::from(terminal));
    let total_units = lifecycle
        .get("total_units")
        .and_then(Value::as_u64)
        .or(terminal.then_some(1));
    let digest_source = json!({
        "phase_key": phase_key,
        "state": state,
        "completed_units": completed_units,
        "total_units": total_units,
        "checkpoint_id": lifecycle.get("checkpoint_id"),
        "poll_ref": lifecycle.get("poll_ref"),
    });
    let progress_digest = format!(
        "sha256:{:x}",
        Sha256::digest(digest_source.to_string().as_bytes())
    );
    json!({
        "schema_version": OPERATION_PROGRESS_SCHEMA_VERSION,
        "phase_key": phase_key,
        "completed_units": completed_units,
        "total_units": total_units,
        "progress_digest": progress_digest,
        "heartbeat_at": lifecycle
            .get("heartbeat_at")
            .or_else(|| lifecycle.get("last_heartbeat_ts"))
            .and_then(Value::as_i64)
            .or(heartbeat_fallback),
        "next_poll_after": lifecycle
            .get("next_poll_after")
            .or_else(|| lifecycle.get("next_check_after")),
        "can_pause": lifecycle
            .get("can_pause")
            .and_then(Value::as_bool)
            .unwrap_or(!terminal && state != "pause_requested"),
        "can_cancel": lifecycle
            .get("can_cancel")
            .and_then(Value::as_bool)
            .unwrap_or(!terminal),
        "progress_kind": lifecycle
            .get("progress_kind")
            .and_then(Value::as_str)
            .filter(|value| matches!(*value, "measured" | "poll_status" | "alive_only"))
            .unwrap_or("alive_only"),
        "detail_ref": lifecycle.get("source").and_then(Value::as_str),
    })
}

pub(crate) fn attach_operation_progress_to_event_payload(payload: &mut Value) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    if object.contains_key("operation_progress") {
        return;
    }
    let lifecycle = object
        .get("task_lifecycle")
        .or_else(|| object.get("lifecycle"))
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| synthetic_lifecycle(object));
    object.insert(
        "operation_progress".to_string(),
        operation_progress_from_lifecycle(&lifecycle, Some(crate::now_ts_u64() as i64)),
    );
}

fn synthetic_lifecycle(object: &Map<String, Value>) -> Value {
    json!({
        "state": object
            .get("state")
            .or_else(|| object.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("running"),
        "phase_key": object
            .get("phase_key")
            .or_else(|| object.get("stage"))
            .and_then(Value::as_str),
        "completed_units": object.get("completed_units"),
        "total_units": object.get("total_units"),
        "progress_kind": object.get("progress_kind").and_then(Value::as_str),
        "source": object.get("source").and_then(Value::as_str),
        "can_pause": object.get("can_pause").and_then(Value::as_bool),
        "can_cancel": object.get("can_cancel").and_then(Value::as_bool),
        "next_poll_after": object
            .get("next_poll_after")
            .or_else(|| object.get("next_check_after")),
    })
}

fn machine_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

#[cfg(test)]
#[path = "long_task_progress_tests.rs"]
mod tests;

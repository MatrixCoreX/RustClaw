use axum::http::{HeaderMap, HeaderValue};
use serde_json::json;

use super::*;

#[test]
fn cursor_prefers_query_and_accepts_last_event_id() {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::HeaderName::from_static("last-event-id"),
        HeaderValue::from_static("41"),
    );
    assert_eq!(requested_cursor(&headers, None), Ok(41));
    assert_eq!(requested_cursor(&headers, Some(42)), Ok(42));
}

#[test]
fn cursor_rejects_non_numeric_header() {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::HeaderName::from_static("last-event-id"),
        HeaderValue::from_static("not-a-cursor"),
    );
    assert_eq!(requested_cursor(&headers, None), Err(()));
}

#[test]
fn terminal_detection_uses_machine_event_kind() {
    assert!(event_is_terminal(&json!({"event_kind":"task_final"})));
    assert!(!event_is_terminal(&json!({
        "event_kind":"tool_finished",
        "payload":{"text":"task_final"}
    })));
}

#[test]
fn cursor_expiry_control_has_no_sequence_id() {
    let value = cursor_expired_control_event("task-a", 5, Some(9), Some(12));
    assert!(value.get("seq").is_none());
    assert_eq!(value["payload"]["replay_mode"], "retained_suffix");
    assert_eq!(value["payload"]["oldest_available_seq"], 9);
}

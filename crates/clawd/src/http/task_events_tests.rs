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
    let value = cursor_expired_control_event("task-a", 5, Some(9), Some(12), "archive");
    assert!(value.get("seq").is_none());
    assert_eq!(value["payload"]["replay_mode"], "available_suffix");
    assert_eq!(value["payload"]["replay_source"], "archive");
    assert_eq!(value["payload"]["oldest_available_seq"], 9);
}

#[test]
fn archive_replay_control_projects_snapshot_without_sequence_id() {
    let snapshot = json!({
        "schema_version": 1,
        "snapshot_seq": 256,
        "source_event_range": {"start_seq": 1, "end_seq": 256}
    });
    let value = archive_replay_control_event("task-a", 40, Some(1), Some(300), Some(&snapshot));
    assert!(value.get("seq").is_none());
    assert_eq!(value["event_kind"], "archive_replay");
    assert_eq!(value["payload"]["replay_mode"], "archive_recovery");
    assert_eq!(value["payload"]["latest_snapshot"]["snapshot_seq"], 256);
}

#[test]
fn event_query_defaults_to_follow_and_accepts_snapshot_mode() {
    let default_query = TaskEventQuery::default();
    assert_eq!(default_query.follow.unwrap_or(true), true);

    let snapshot_query = TaskEventQuery {
        cursor: Some(7),
        follow: Some(false),
    };
    assert_eq!(snapshot_query.cursor, Some(7));
    assert_eq!(snapshot_query.follow, Some(false));
}

#[tokio::test]
async fn snapshot_mode_drains_more_than_one_archive_page() {
    const EVENT_COUNT: u64 = 1_026;
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task_id = uuid::Uuid::new_v4();
    let user_key = "task-event-archive-test-key";
    {
        let db = state.core.db.get().expect("get db");
        db.execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at)
             VALUES (?1, 'admin', 1, '1')",
            rusqlite::params![user_key],
        )
        .expect("insert auth key");
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, channel, kind,
                payload_json, status, created_at, updated_at
             ) VALUES (?1, ?2, 7, ?3, 'ui', 'ask', '{}', 'running', '1', '1')",
            rusqlite::params![
                task_id.to_string(),
                crate::stable_i64_from_key(user_key),
                user_key
            ],
        )
        .expect("insert task");
    }
    for index in 0..EVENT_COUNT {
        crate::task_event_transport::publish_event(
            &state,
            &task_id.to_string(),
            "task_observation",
            json!({"index": index}),
        )
        .expect("publish event");
    }
    let mut headers = HeaderMap::new();
    headers.insert("x-rustclaw-key", HeaderValue::from_static(user_key));

    let response = stream_task_events(
        State(state),
        headers,
        Path(task_id),
        Query(TaskEventQuery {
            cursor: Some(0),
            follow: Some(false),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("read SSE snapshot body");
    let text = String::from_utf8(body.to_vec()).expect("decode SSE body");
    assert!(text.contains("\"seq\":1"));
    assert!(text.contains(&format!("\"seq\":{}", EVENT_COUNT + 1)));
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("data:"))
            .count(),
        (EVENT_COUNT + 1) as usize
    );
}

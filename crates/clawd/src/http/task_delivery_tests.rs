use super::*;
use claw_core::channel_delivery::ChannelDeliverySource;

fn record(status: &str, result_json: Option<Value>) -> TaskDeliveryRecord {
    let payload = serde_json::json!({
        "channel_ingress": {"locale": "zh-CN"}
    });
    TaskDeliveryRecord {
        task: crate::ClaimedTask {
            claim_attempt: 1,
            task_id: "terminal-task".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: Some("key".to_string()),
            channel: "telegram".to_string(),
            external_user_id: Some("1".to_string()),
            external_chat_id: Some("2".to_string()),
            kind: "ask".to_string(),
            payload_json: payload.to_string(),
        },
        status: status.to_string(),
        result_json,
        error_text: Some("private provider detail".to_string()),
    }
}

#[test]
fn successful_terminal_content_prefers_structured_messages() {
    let state = AppState::test_default_with_fixture_provider();
    let record = record(
        "succeeded",
        Some(serde_json::json!({
            "messages": ["first", "second"],
            "text": "fallback"
        })),
    );
    let payload: Value = serde_json::from_str(&record.task.payload_json).unwrap();
    let (text, notice) =
        terminal_delivery_content(&state, &record, &payload, TaskStatus::Succeeded);
    assert_eq!(text, "first\n\nsecond");
    assert!(notice.is_none());
}

#[test]
fn failed_terminal_content_uses_common_locale_not_raw_error() {
    let state = AppState::test_default_with_fixture_provider();
    let record = record("failed", None);
    let payload: Value = serde_json::from_str(&record.task.payload_json).unwrap();
    let (text, notice) = terminal_delivery_content(&state, &record, &payload, TaskStatus::Failed);
    assert_eq!(text, "请求未能完成，请重试。");
    assert!(!text.contains("private provider detail"));
    let notice = notice.expect("failure notice");
    assert_eq!(notice.error_code.as_deref(), Some("task.failed"));
    assert_eq!(notice.message_key, "channel.task.failed");
    assert!(notice.retryable);
}

#[test]
fn daemon_request_rejects_schedule_and_proactive_sources() {
    for source in [
        ChannelDeliverySource::ScheduledTask,
        ChannelDeliverySource::ProactiveNotice,
    ] {
        let request = ChannelTaskDeliveryRequest::daemon(source);
        assert!(request.validate().is_err());
    }
}

#[test]
fn terminal_content_projection_separates_text_and_media_without_rewriting_tokens() {
    let full = "result text\nIMAGE_FILE:/tmp/result.png\nVIDEO_URL:https://example.test/v.mp4";
    assert_eq!(
        project_terminal_delivery_content(full, ChannelTaskDeliveryContent::TextOnly),
        "result text"
    );
    assert_eq!(
        project_terminal_delivery_content(full, ChannelTaskDeliveryContent::MediaOnly),
        "IMAGE_FILE:/tmp/result.png\nVIDEO_URL:https://example.test/v.mp4"
    );
    assert_eq!(
        project_terminal_delivery_content(full, ChannelTaskDeliveryContent::Full),
        full
    );
}

#[test]
fn interrupted_terminal_content_uses_shared_resume_notice() {
    let state = AppState::test_default_with_fixture_provider();
    let record = record(
        "failed",
        Some(serde_json::json!({"resume_context": {"resume_context_id": "ctx-1"}})),
    );
    let payload: Value = serde_json::from_str(&record.task.payload_json).unwrap();
    let (text, notice) = terminal_delivery_content(&state, &record, &payload, TaskStatus::Failed);
    assert!(text.contains("直接回复"));
    assert!(!text.contains("private provider detail"));
    let notice = notice.expect("resume notice");
    assert_eq!(
        notice.error_code.as_deref(),
        Some("task.resume_interrupted")
    );
    assert_eq!(notice.message_key, "channel.task.resume_interrupted");
    assert!(notice.retryable);
}

#[test]
fn delivery_authorization_requires_the_tasks_exact_active_key() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let db = state.core.db.get().expect("db connection");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at) VALUES (?1, 'user', 1, '1')",
        rusqlite::params!["key"],
    )
    .expect("insert active key");
    drop(db);
    let record = record("succeeded", Some(serde_json::json!({"text": "done"})));

    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        "key".parse().expect("key header"),
    );
    assert!(authorized_delivery_request(&state, &headers, &record));

    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        "other".parse().expect("other header"),
    );
    assert!(!authorized_delivery_request(&state, &headers, &record));

    let db = state.core.db.get().expect("db connection");
    db.execute(
        "UPDATE auth_keys SET enabled = 0 WHERE user_key = 'key'",
        [],
    )
    .expect("disable key");
    drop(db);
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        "key".parse().expect("key header"),
    );
    assert!(!authorized_delivery_request(&state, &headers, &record));
}

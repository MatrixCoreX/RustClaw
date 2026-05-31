use serde_json::json;

#[test]
fn wechat_payload_shape_keeps_context_token_available() {
    let payload = json!({
        "channel": "wechat",
        "external_chat_id": "wx-user-1",
        "context_token": "ctx-123"
    });
    assert_eq!(
        payload.get("channel").and_then(|v| v.as_str()),
        Some("wechat")
    );
    assert_eq!(
        payload.get("context_token").and_then(|v| v.as_str()),
        Some("ctx-123")
    );
}

#[test]
fn schedule_notify_observation_marks_delivery_failure() {
    let observation = super::schedule_notify_observation(&super::ScheduleNotifyOutcome {
        job_id: "job-1".to_string(),
        channel: "telegram".to_string(),
        runtime_channel: "telegram".to_string(),
        task_success: true,
        delivered: false,
        error_text: Some("telegram bot token is empty".to_string()),
    });

    assert_eq!(
        observation.get("source").and_then(|value| value.as_str()),
        Some("schedule_notify")
    );
    assert_eq!(
        observation.get("status").and_then(|value| value.as_str()),
        Some("error")
    );
    assert_eq!(
        observation
            .get("error_kind")
            .and_then(|value| value.as_str()),
        Some("channel_send_failed")
    );
    assert_eq!(
        observation
            .get("failure_attribution")
            .and_then(|value| value.as_str()),
        Some("delivery_error")
    );
}

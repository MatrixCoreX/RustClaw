use super::*;

#[test]
fn notice_round_trip_preserves_machine_contract() {
    let mut notice = ChannelNotice::error(
        "channel.delivery_failed",
        "provider.rate_limited",
        "channel.error.retry_later",
        true,
    );
    notice
        .params
        .insert("task_id".to_string(), "task-1".to_string());
    notice.next_actions.push(ChannelNoticeNextAction {
        kind: ChannelNoticeActionKind::Retry,
        message_key: Some("channel.action.retry".to_string()),
        params: BTreeMap::new(),
    });
    notice.diagnostic_id = Some("diag:123".to_string());

    notice.validate().expect("valid notice");
    let encoded = serde_json::to_string(&notice).expect("encode notice");
    let decoded: ChannelNotice = serde_json::from_str(&encoded).expect("decode notice");
    assert_eq!(decoded, notice);
}

#[test]
fn notice_rejects_user_text_in_machine_fields() {
    let notice = ChannelNotice::error(
        "channel.delivery_failed",
        "Please retry later",
        "channel.error.retry_later",
        true,
    );

    assert_eq!(
        notice.validate(),
        Err(ChannelNoticeValidationError::InvalidErrorCode)
    );
}

#[test]
fn status_notice_does_not_invent_error_state() {
    let notice = ChannelNotice::status(
        "channel.working",
        "channel.notice.working",
        ChannelNoticeSeverity::Info,
    );

    notice.validate().expect("valid status notice");
    assert_eq!(notice.error_code, None);
    assert!(!notice.retryable);
}

#[test]
fn notice_rejects_private_diagnostics_in_public_params() {
    for (name, value) in [
        ("error", "provider failed"),
        ("detail", "request rejected"),
        ("filename", "/home/user/private.txt"),
        ("reason", "Authorization: Bearer secret"),
        ("reason", "Traceback (most recent call last): boom"),
    ] {
        let mut notice = ChannelNotice::error(
            "channel.delivery_failed",
            "provider.unavailable",
            "channel.error.retry_later",
            true,
        );
        notice.params.insert(name.to_string(), value.to_string());
        assert_eq!(
            notice.validate(),
            Err(ChannelNoticeValidationError::InvalidParam),
            "unsafe public param unexpectedly accepted: {name}={value}"
        );
    }
}

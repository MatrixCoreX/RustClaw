use super::*;

#[test]
fn conversation_scope_isolated_by_account_channel_and_peer() {
    let base =
        WechatConversationScope::new("account-a", "wechat_ilink", "peer-a").expect("base scope");
    let scopes = [
        WechatConversationScope::new("account-b", "wechat_ilink", "peer-a").expect("account scope"),
        WechatConversationScope::new("account-a", "wechat-alt", "peer-a").expect("channel scope"),
        WechatConversationScope::new("account-a", "wechat_ilink", "peer-b").expect("peer scope"),
    ];

    for scope in scopes {
        assert_ne!(base.storage_key(), scope.storage_key());
    }
    assert_eq!(base.account_id(), "account-a");
    assert_eq!(base.channel(), "wechat_ilink");
    assert_eq!(base.peer_id(), "peer-a");
}

#[test]
fn official_item_types_keep_exact_fields() {
    let media = || {
        WechatCdnMedia::encrypted("download-param".to_string(), "base64-key".to_string())
            .expect("cdn media")
    };
    let cases = [
        (
            WechatMessageItem::image(media(), 32).expect("image"),
            MESSAGE_ITEM_IMAGE,
            "image_item",
        ),
        (
            WechatMessageItem::voice_silk(media(), 16_000, 1_200).expect("voice"),
            MESSAGE_ITEM_VOICE,
            "voice_item",
        ),
        (
            WechatMessageItem::file(media(), "report.pdf", 31).expect("file"),
            MESSAGE_ITEM_FILE,
            "file_item",
        ),
        (
            WechatMessageItem::video(media(), 48).expect("video"),
            MESSAGE_ITEM_VIDEO,
            "video_item",
        ),
    ];

    for (item, expected_type, expected_field) in cases {
        let value = serde_json::to_value(item).expect("serialize item");
        assert_eq!(value["type"], expected_type);
        assert!(value.get(expected_field).is_some(), "{value}");
        assert_eq!(value[expected_field]["media"]["encrypt_type"], 1);
    }
}

#[test]
fn generating_and_finish_requests_share_run_and_context() {
    let generating = WechatSendMessageRequest::generating(
        "peer",
        "context",
        "client-generating",
        "run-1",
        "test",
    )
    .expect("generating request");
    let finish = WechatSendMessageRequest::finish(
        "peer",
        "context",
        "client-finish",
        Some("run-1".to_string()),
        WechatMessageItem::text("done").expect("text item"),
        "test",
    )
    .expect("finish request");

    assert_eq!(generating.msg.message_state, MESSAGE_STATE_GENERATING);
    assert!(generating.msg.item_list.is_none());
    assert_eq!(finish.msg.message_state, MESSAGE_STATE_FINISH);
    assert_eq!(finish.msg.item_list.as_ref().map(Vec::len), Some(1));
    assert_eq!(generating.msg.context_token, finish.msg.context_token);
    assert_eq!(generating.msg.run_id, finish.msg.run_id);

    let progress = WechatSendMessageRequest::generating_with_item(
        "peer",
        "context",
        "client-progress",
        "run-1",
        WechatMessageItem::text("working").expect("progress text"),
        "test",
    )
    .expect("progress request");
    assert_eq!(progress.msg.message_state, MESSAGE_STATE_GENERATING);
    assert_eq!(progress.msg.item_list.as_ref().map(Vec::len), Some(1));
    assert_eq!(progress.msg.run_id, generating.msg.run_id);
}

#[test]
fn send_request_rejects_missing_context_token() {
    let error = WechatSendMessageRequest::generating("peer", " ", "client", "run", "test")
        .expect_err("missing context must fail");
    assert_eq!(error, "wechat_context_token_missing");
}

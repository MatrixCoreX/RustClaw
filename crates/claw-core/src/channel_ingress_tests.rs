use serde_json::json;

use super::{
    default_adapter_for_channel, default_reply_target, ChannelIngressAttachment,
    ChannelIngressEnvelope, ChannelReplyTarget, ChannelReplyTargetKind,
    CHANNEL_INGRESS_SCHEMA_VERSION,
};
use crate::types::ChannelKind;

#[test]
fn envelope_round_trip_preserves_machine_owned_channel_context() {
    let mut envelope = ChannelIngressEnvelope::new(ChannelKind::Wechat, "wechat_ilink")
        .with_external_ids("wx-user", "wx-chat")
        .with_message_id("message-1")
        .with_reply_target(ChannelReplyTarget::user("wx-user"))
        .with_locale("zh-CN")
        .with_context_token("context-1");
    envelope.bound_user_id = Some(7);
    envelope.conversation_chat_id = Some(8);
    envelope.attachments.push(ChannelIngressAttachment {
        kind: "image".to_string(),
        path: "data/inbox/image.jpg".to_string(),
        mime_type: Some("image/jpeg".to_string()),
        size: Some(42),
    });

    let value = serde_json::to_value(&envelope).expect("serialize ingress envelope");
    assert_eq!(value["schema_version"], CHANNEL_INGRESS_SCHEMA_VERSION);
    assert_eq!(value["reply_target"]["kind"], "user");
    assert_eq!(value["attachments"][0]["path"], "data/inbox/image.jpg");

    let decoded: ChannelIngressEnvelope =
        serde_json::from_value(value).expect("deserialize ingress envelope");
    assert_eq!(decoded, envelope);
}

#[test]
fn optional_fields_are_omitted_from_compact_wire_shape() {
    let value = serde_json::to_value(ChannelIngressEnvelope::new(
        ChannelKind::Telegram,
        "telegram_bot",
    ))
    .expect("serialize minimal ingress envelope");

    assert_eq!(
        value,
        json!({
            "schema_version": 1,
            "channel": "telegram",
            "adapter": "telegram_bot"
        })
    );
}

#[test]
fn legacy_submit_request_remains_wire_compatible() {
    let request: crate::types::SubmitTaskRequest = serde_json::from_value(json!({
        "user_id": 7,
        "chat_id": 8,
        "channel": "telegram",
        "kind": "ask",
        "payload": {"text": "hello"}
    }))
    .expect("deserialize legacy submit request");

    assert!(request.ingress.is_none());
    assert_eq!(request.channel, Some(ChannelKind::Telegram));
}

#[test]
fn channel_defaults_choose_the_delivery_address_shape() {
    assert_eq!(
        default_adapter_for_channel(ChannelKind::Whatsapp),
        "whatsapp_cloud"
    );
    assert_eq!(
        default_reply_target(ChannelKind::Whatsapp, Some("user-1"), Some("chat-1"))
            .expect("whatsapp reply target")
            .kind,
        ChannelReplyTargetKind::User
    );
    assert_eq!(
        default_reply_target(ChannelKind::Feishu, Some("user-1"), Some("chat-1"))
            .expect("feishu reply target")
            .kind,
        ChannelReplyTargetKind::Chat
    );
}

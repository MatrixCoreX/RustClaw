use super::*;
use crate::channel_ingress::ChannelReplyTarget;

fn delivery() -> ChannelDeliveryEnvelope {
    ChannelDeliveryEnvelope {
        schema_version: CHANNEL_DELIVERY_SCHEMA_VERSION,
        delivery_id: "delivery:task-1:final".to_string(),
        task_id: Some("task-1".to_string()),
        source: ChannelDeliverySource::BackgroundCompletion,
        channel: ChannelKind::Wechat,
        adapter: "wechat_ilink".to_string(),
        reply_target: ChannelReplyTarget::user("peer-1"),
        locale: "zh-CN".to_string(),
        conversation_window: ChannelConversationWindow {
            state: ChannelConversationWindowState::Open,
            expires_at_ts: Some(200),
            context_token: Some("context-1".to_string()),
        },
        idempotency_key: "wechat:peer-1:task-1:final".to_string(),
        text_segments: vec![ChannelTextSegment {
            text: "result".to_string(),
            format: ChannelTextFormat::Plain,
        }],
        artifacts: vec![ChannelArtifactRef {
            artifact_ref: "artifact:task-1:image-1".to_string(),
            kind: ChannelArtifactKind::Image,
            mime_type: Some("image/png".to_string()),
            display_name: Some("image.png".to_string()),
            size: Some(10),
        }],
        previews: vec![ChannelArtifactPreview {
            artifact_ref: "artifact:task-1:image-1".to_string(),
            preview_artifact_ref: "artifact:task-1:image-1-preview".to_string(),
            mime_type: Some("image/jpeg".to_string()),
        }],
        notice: None,
    }
}

fn receipt(status: ChannelDeliveryStatus, retryable: bool) -> ChannelDeliveryReceipt {
    ChannelDeliveryReceipt {
        schema_version: CHANNEL_DELIVERY_RECEIPT_SCHEMA_VERSION,
        delivery_id: "delivery:task-1:final".to_string(),
        idempotency_key: "wechat:peer-1:task-1:final".to_string(),
        channel: ChannelKind::Wechat,
        adapter: "wechat_ilink".to_string(),
        status,
        provider_message_ids: Vec::new(),
        parts: Vec::new(),
        error_code: None,
        message_key: None,
        diagnostic_id: None,
        provider_error_code: None,
        retryable,
        updated_at_ts: 100,
    }
}

#[test]
fn delivery_envelope_covers_all_shared_sources_and_payload_shapes() {
    for source in [
        ChannelDeliverySource::ImmediateDaemon,
        ChannelDeliverySource::BackgroundCompletion,
        ChannelDeliverySource::ScheduledTask,
        ChannelDeliverySource::ProactiveNotice,
    ] {
        let mut value = delivery();
        value.source = source;
        value.validate().expect("valid delivery envelope");
        let encoded = serde_json::to_string(&value).expect("encode delivery");
        let decoded: ChannelDeliveryEnvelope =
            serde_json::from_str(&encoded).expect("decode delivery");
        assert_eq!(decoded, value);
    }
}

#[test]
fn task_delivery_request_defaults_to_full_content_and_restricts_daemon_sources() {
    let decoded: ChannelTaskDeliveryRequest = serde_json::from_value(serde_json::json!({
        "schema_version": CHANNEL_TASK_DELIVERY_REQUEST_SCHEMA_VERSION,
        "source": "immediate_daemon"
    }))
    .expect("decode request");
    assert_eq!(decoded.content, ChannelTaskDeliveryContent::Full);
    decoded.validate().expect("valid daemon request");

    let media_only = ChannelTaskDeliveryRequest::daemon_with_content(
        ChannelDeliverySource::BackgroundCompletion,
        ChannelTaskDeliveryContent::MediaOnly,
    );
    media_only.validate().expect("valid media projection");
    assert_eq!(media_only.content, ChannelTaskDeliveryContent::MediaOnly);
}

#[test]
fn delivery_requires_payload_and_preview_parent() {
    let mut empty = delivery();
    empty.text_segments.clear();
    empty.artifacts.clear();
    empty.previews.clear();
    assert_eq!(
        empty.validate(),
        Err(ChannelDeliveryValidationError::EmptyDelivery)
    );

    let mut orphan = delivery();
    orphan.previews[0].artifact_ref = "artifact:task-1:missing".to_string();
    assert_eq!(
        orphan.validate(),
        Err(ChannelDeliveryValidationError::InvalidPreview)
    );

    let mut overwriting_preview = delivery();
    overwriting_preview.previews[0].preview_artifact_ref =
        overwriting_preview.previews[0].artifact_ref.clone();
    assert_eq!(
        overwriting_preview.validate(),
        Err(ChannelDeliveryValidationError::InvalidPreview)
    );
}

#[test]
fn transport_metadata_never_becomes_assistant_success_history() {
    for source in [
        ChannelDeliverySource::ImmediateDaemon,
        ChannelDeliverySource::BackgroundCompletion,
        ChannelDeliverySource::ScheduledTask,
    ] {
        let mut value = delivery();
        value.source = source;
        assert_eq!(
            value.history_disposition(),
            ChannelDeliveryHistoryDisposition::AssistantResult
        );
    }

    let mut proactive = delivery();
    proactive.source = ChannelDeliverySource::ProactiveNotice;
    assert_eq!(
        proactive.history_disposition(),
        ChannelDeliveryHistoryDisposition::TransportOnly
    );

    let mut notice = delivery();
    notice.notice = Some(crate::channel_notice::ChannelNotice::status(
        "channel.typing",
        "channel.msg.typing",
        crate::channel_notice::ChannelNoticeSeverity::Info,
    ));
    assert_eq!(
        notice.history_disposition(),
        ChannelDeliveryHistoryDisposition::TransportOnly
    );

    assert_eq!(
        receipt(ChannelDeliveryStatus::Accepted, false).history_disposition(),
        ChannelDeliveryHistoryDisposition::TransportOnly
    );
}

#[test]
fn receipt_states_separate_accepted_delivered_failed_and_partial() {
    let accepted = receipt(ChannelDeliveryStatus::Accepted, false);
    accepted.validate().expect("accepted receipt");

    let mut delivered = receipt(ChannelDeliveryStatus::Delivered, false);
    delivered
        .provider_message_ids
        .push("provider-1".to_string());
    delivered.validate().expect("delivered receipt");

    let mut failed = receipt(ChannelDeliveryStatus::Failed, true);
    failed.error_code = Some("provider.rate_limited".to_string());
    failed.message_key = Some("channel.error.provider_rate_limited".to_string());
    failed.diagnostic_id = Some("diag:1".to_string());
    failed.validate().expect("failed receipt");

    let mut partial = receipt(ChannelDeliveryStatus::Partial, true);
    partial.error_code = Some("provider.partial_failure".to_string());
    partial.message_key = Some("channel.error.provider_unavailable".to_string());
    partial.diagnostic_id = Some("diag:2".to_string());
    partial.parts = vec![
        ChannelDeliveryPartReceipt {
            part_index: 0,
            status: ChannelDeliveryStatus::Delivered,
            provider_message_id: Some("provider-1".to_string()),
            error_code: None,
        },
        ChannelDeliveryPartReceipt {
            part_index: 1,
            status: ChannelDeliveryStatus::Failed,
            provider_message_id: None,
            error_code: Some("provider.upload_failed".to_string()),
        },
    ];
    partial.validate().expect("partial receipt");
}

#[test]
fn retry_decision_queries_receipt_before_resending() {
    assert_eq!(
        delivery_retry_decision(None),
        ChannelDeliveryRetryDecision::SendNew
    );

    let accepted = receipt(ChannelDeliveryStatus::Accepted, false);
    assert_eq!(
        delivery_retry_decision(Some(&accepted)),
        ChannelDeliveryRetryDecision::QueryProviderReceipt
    );

    let mut delivered = receipt(ChannelDeliveryStatus::Delivered, false);
    delivered
        .provider_message_ids
        .push("provider-1".to_string());
    assert_eq!(
        delivery_retry_decision(Some(&delivered)),
        ChannelDeliveryRetryDecision::AlreadyDelivered
    );

    let mut retryable_partial = receipt(ChannelDeliveryStatus::Partial, true);
    retryable_partial.error_code = Some("provider.partial_failure".to_string());
    retryable_partial.message_key = Some("channel.error.provider_unavailable".to_string());
    retryable_partial.diagnostic_id = Some("diag:2".to_string());
    assert_eq!(
        delivery_retry_decision(Some(&retryable_partial)),
        ChannelDeliveryRetryDecision::RetryFailedParts
    );
}

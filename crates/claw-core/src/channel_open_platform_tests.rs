use super::*;

#[test]
fn regional_contracts_share_limits_without_sharing_runtime_namespaces() {
    let feishu = open_platform_contract(OpenPlatformRegion::Feishu);
    let lark = open_platform_contract(OpenPlatformRegion::Lark);

    assert_ne!(feishu.channel, lark.channel);
    assert_ne!(feishu.adapter, lark.adapter);
    assert_ne!(feishu.source_adapter, lark.source_adapter);
    assert_ne!(feishu.rate_bucket_namespace, lark.rate_bucket_namespace);
    assert_ne!(feishu.receipt_namespace, lark.receipt_namespace);
    assert!(feishu.message_source_ref.contains("open.feishu.cn"));
    assert!(lark.message_source_ref.contains("open.larksuite.com"));
}

#[test]
fn text_chunks_use_serialized_utf8_bytes_and_keep_character_boundaries() {
    let text = "🙂\"换行\n".repeat(40_000);
    let chunks = chunk_open_platform_text(&text, usize::MAX).expect("byte-safe chunks");
    assert!(chunks.len() > 1);
    assert_eq!(chunks.concat(), text);
    for chunk in chunks {
        let content = serde_json::json!({ "text": chunk }).to_string();
        assert!(content.len() <= OPEN_PLATFORM_TEXT_CONTENT_MAX_BYTES);
        validate_open_platform_content(OpenPlatformMessageType::Text, &content)
            .expect("chunk must remain within provider limit");
    }
}

#[test]
fn structured_content_uses_the_smaller_card_and_post_limit() {
    let oversized = "x".repeat(OPEN_PLATFORM_STRUCTURED_CONTENT_MAX_BYTES + 1);
    for message_type in [
        OpenPlatformMessageType::Post,
        OpenPlatformMessageType::Interactive,
    ] {
        let error = validate_open_platform_content(message_type, &oversized)
            .expect_err("structured content must fail before provider call");
        assert_eq!(
            error.error_code(),
            "channel_open_platform_content_too_large"
        );
        assert_eq!(error.max_bytes, OPEN_PLATFORM_STRUCTURED_CONTENT_MAX_BYTES);
    }
}

#[test]
fn provider_message_tokens_are_a_closed_set() {
    for (token, expected) in [
        ("text", OpenPlatformMessageType::Text),
        ("interactive", OpenPlatformMessageType::Interactive),
        ("media", OpenPlatformMessageType::Media),
        ("file", OpenPlatformMessageType::File),
    ] {
        assert_eq!(
            OpenPlatformMessageType::from_provider_token(token),
            Some(expected)
        );
    }
    assert_eq!(OpenPlatformMessageType::from_provider_token("video"), None);
}

#[test]
fn rate_and_receipt_keys_are_region_scoped() {
    assert_ne!(
        scoped_open_platform_rate_bucket(OpenPlatformRegion::Feishu, "chat"),
        scoped_open_platform_rate_bucket(OpenPlatformRegion::Lark, "chat")
    );
    assert_ne!(
        scoped_open_platform_receipt_key(OpenPlatformRegion::Feishu, "delivery"),
        scoped_open_platform_receipt_key(OpenPlatformRegion::Lark, "delivery")
    );
}

#[test]
fn target_rate_reservations_are_spaced_and_region_scoped() {
    let now = Instant::now();
    let mut buckets = HashMap::new();
    let feishu_key = scoped_open_platform_rate_bucket(OpenPlatformRegion::Feishu, "chat");
    let lark_key = scoped_open_platform_rate_bucket(OpenPlatformRegion::Lark, "chat");

    assert_eq!(
        OpenPlatformTargetRateLimiter::reserve_at(&mut buckets, feishu_key.clone(), now),
        Duration::ZERO
    );
    assert_eq!(
        OpenPlatformTargetRateLimiter::reserve_at(&mut buckets, feishu_key, now),
        Duration::from_millis(OPEN_PLATFORM_TARGET_MIN_INTERVAL_MILLIS)
    );
    assert_eq!(
        OpenPlatformTargetRateLimiter::reserve_at(&mut buckets, lark_key, now),
        Duration::ZERO
    );
}

#[test]
fn provider_codes_override_legacy_http_400_classification() {
    for (code, expected, retryable) in [
        ("230020", ChannelProviderFailureClass::RateLimited, true),
        (
            "230018",
            ChannelProviderFailureClass::PermissionDenied,
            false,
        ),
        ("230019", ChannelProviderFailureClass::TargetNotFound, false),
        (
            "99991403",
            ChannelProviderFailureClass::QuotaExhausted,
            false,
        ),
        (
            "234006",
            ChannelProviderFailureClass::PayloadRejected,
            false,
        ),
    ] {
        let error = open_platform_provider_error(
            OpenPlatformRegion::Lark,
            "send_message",
            400,
            &format!(r#"{{"code":{code},"msg":"private prose"}}"#),
        );
        assert_eq!(error.failure_class, expected);
        assert_eq!(error.retryable, retryable);
        assert_eq!(error.provider_error_code.as_deref(), Some(code));
        assert!(!error.to_string().contains("private prose"));
    }
}

#[test]
fn unknown_provider_code_falls_back_to_http_status() {
    let error = open_platform_provider_error(
        OpenPlatformRegion::Feishu,
        "send_message",
        503,
        r#"{"code":987654321,"msg":"private prose"}"#,
    );
    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::ProviderUnavailable
    );
    assert!(error.retryable);
    assert!(!error.to_string().contains("private prose"));
}

#[tokio::test]
async fn token_cache_single_flights_refresh_and_keeps_token_private() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let cache = Arc::new(OpenPlatformTokenCache::default());
    let refreshes = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let cache = cache.clone();
        let refreshes = refreshes.clone();
        tasks.push(tokio::spawn(async move {
            cache
                .token_or_refresh(100, || async move {
                    refreshes.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    Ok::<_, ()>(("private-token".to_string(), 7200))
                })
                .await
        }));
    }
    for task in tasks {
        assert_eq!(
            task.await.expect("join cache task"),
            Ok("private-token".to_string())
        );
    }
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
}

#[test]
fn process_token_caches_are_scoped_by_region_base_and_app() {
    let feishu = process_open_platform_token_cache(
        OpenPlatformRegion::Feishu,
        "https://provider.example.test/",
        "app-a",
    );
    let feishu_same = process_open_platform_token_cache(
        OpenPlatformRegion::Feishu,
        "https://provider.example.test",
        "app-a",
    );
    let lark = process_open_platform_token_cache(
        OpenPlatformRegion::Lark,
        "https://provider.example.test",
        "app-a",
    );
    let another_app = process_open_platform_token_cache(
        OpenPlatformRegion::Feishu,
        "https://provider.example.test",
        "app-b",
    );
    assert!(std::sync::Arc::ptr_eq(&feishu, &feishu_same));
    assert!(!std::sync::Arc::ptr_eq(&feishu, &lark));
    assert!(!std::sync::Arc::ptr_eq(&feishu, &another_app));
}

#[test]
fn media_plan_keeps_upload_and_message_types_consistent() {
    let image = plan_open_platform_media(
        OpenPlatformRegion::Feishu,
        OpenPlatformOutboundMediaKind::Image,
        Path::new("image.png"),
        1024,
    );
    assert_eq!(image.upload_endpoint, OpenPlatformUploadEndpoint::Image);
    assert_eq!(image.form_file_type, "message");
    assert_eq!(image.message_type, OpenPlatformMessageType::Image);
    assert_eq!(image.key_name, "image_key");

    let large_image = plan_open_platform_media(
        OpenPlatformRegion::Lark,
        OpenPlatformOutboundMediaKind::Image,
        Path::new("image.png"),
        11 * 1024 * 1024,
    );
    assert_eq!(
        large_image.upload_endpoint,
        OpenPlatformUploadEndpoint::File
    );
    assert_eq!(large_image.message_type, OpenPlatformMessageType::File);

    let video = plan_open_platform_media(
        OpenPlatformRegion::Lark,
        OpenPlatformOutboundMediaKind::Video,
        Path::new("video.MP4"),
        1024,
    );
    assert_eq!(video.form_file_type, "mp4");
    assert_eq!(video.message_type, OpenPlatformMessageType::Media);

    let opus = plan_open_platform_media(
        OpenPlatformRegion::Feishu,
        OpenPlatformOutboundMediaKind::Audio,
        Path::new("voice.opus"),
        1024,
    );
    assert_eq!(opus.form_file_type, "opus");
    assert_eq!(opus.message_type, OpenPlatformMessageType::Audio);
}

#[test]
fn response_decoder_rejects_success_http_with_provider_error() {
    let error = decode_open_platform_response(
        OpenPlatformRegion::Feishu,
        "send_message",
        200,
        r#"{"code":230020,"msg":"private prose"}"#,
    )
    .expect_err("provider code must override successful HTTP status");
    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::RateLimited
    );
    assert!(!error.to_string().contains("private prose"));
}

#[test]
fn message_id_decoder_requires_stable_machine_identifier() {
    let body = r#"{"code":0,"data":{"message_id":"om_fixture-123"}}"#;
    assert_eq!(
        open_platform_message_id(OpenPlatformRegion::Lark, "send_message", 200, body)
            .expect("message id"),
        "om_fixture-123"
    );
    let error = open_platform_message_id(
        OpenPlatformRegion::Lark,
        "send_message",
        200,
        r#"{"code":0,"data":{}}"#,
    )
    .expect_err("missing message id must fail closed");
    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::InvalidResponse
    );
}

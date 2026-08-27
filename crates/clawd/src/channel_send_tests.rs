use super::*;

#[test]
fn wechat_delivery_part_ids_are_stable_and_part_scoped() {
    let first = wechat_delivery_client_id("delivery:task-1:terminal", "text", 0);
    assert_eq!(
        first,
        wechat_delivery_client_id("delivery:task-1:terminal", "text", 0)
    );
    assert_ne!(
        first,
        wechat_delivery_client_id("delivery:task-1:terminal", "text", 1)
    );
    assert_ne!(
        first,
        wechat_delivery_client_id("delivery:task-1:terminal", "image", 0)
    );
}

#[test]
fn wechat_part_failure_is_retained_without_suppressing_later_parts() {
    let mut first_error = None;
    let mut accepted = Vec::new();
    for (index, result) in [Ok(()), Err("video rejected".to_string()), Ok(())]
        .into_iter()
        .enumerate()
    {
        if record_wechat_part_result(&mut first_error, result, "fixture", index) {
            accepted.push(index);
        }
    }

    assert_eq!(accepted, vec![0, 2]);
    assert_eq!(first_error.as_deref(), Some("video rejected"));
}

#[tokio::test]
async fn channel_send_progress_survives_a_later_part_failure() {
    let (result, provider_message_ids) = capture_channel_send_progress(async {
        record_provider_message_id("provider-part-1");
        Result::<(), String>::Err("later part failed".to_string())
    })
    .await;

    assert_eq!(result, Err("later part failed".to_string()));
    assert_eq!(provider_message_ids, vec!["provider-part-1"]);
}
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[test]
fn telegram_success_response_projects_stable_provider_message_id() {
    let body = r#"{"ok":true,"result":{"message_id":12345,"text":"private reply"}}"#;
    assert_eq!(
        telegram_message_id("send_text", body).as_deref(),
        Ok("12345")
    );
}

#[test]
fn telegram_url_buttons_are_extracted_for_the_shared_sender() {
    let (text, buttons) = extract_telegram_url_buttons(
        "Result\nBUTTON: Open：https://a.example\nBUTTON: Open：https://b.example",
    );
    assert_eq!(text, "Result");
    assert_eq!(
        buttons,
        vec![
            ("Open".to_string(), "https://a.example".to_string()),
            ("Open 2".to_string(), "https://b.example".to_string()),
        ]
    );

    let (text, buttons) = extract_telegram_url_buttons("BUTTON: Open:not-a-url");
    assert_eq!(text, "BUTTON: Open:not-a-url");
    assert!(buttons.is_empty());
}

#[test]
fn telegram_success_without_message_id_is_a_redacted_invalid_response() {
    let body = r#"{"ok":true,"result":{"text":"private reply"}}"#;
    let error = telegram_message_id("send_text", body).expect_err("missing id must fail");
    let decoded = ChannelProviderError::decode(&error).expect("machine provider error");
    assert_eq!(
        decoded.failure_class,
        claw_core::channel_provider_error::ChannelProviderFailureClass::InvalidResponse
    );
    assert!(!error.contains("private reply"));
}

#[test]
fn telegram_http_rate_limit_keeps_retry_after_without_response_prose() {
    let body = r#"{"ok":false,"error_code":429,"description":"private reply","parameters":{"retry_after":11}}"#;
    let encoded = provider_http_error(
        "telegram_bot",
        "send_text",
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        body,
    );
    let decoded = ChannelProviderError::decode(&encoded).expect("machine provider error");
    assert_eq!(decoded.retry_after_seconds, Some(11));
    assert!(decoded.retryable);
    assert!(!encoded.contains("private reply"));
}

#[test]
fn open_platform_provider_codes_override_legacy_http_status_for_each_region() {
    for source_adapter in ["feishu_open_platform", "lark_open_platform"] {
        let encoded = provider_http_error(
            source_adapter,
            "send_text",
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"code":230020,"msg":"private reply"}"#,
        );
        let decoded = ChannelProviderError::decode(&encoded).expect("machine provider error");
        assert_eq!(
            decoded.failure_class,
            claw_core::channel_provider_error::ChannelProviderFailureClass::RateLimited
        );
        assert_eq!(decoded.provider_error_code.as_deref(), Some("230020"));
        assert!(decoded.retryable);
        assert!(!encoded.contains("private reply"));
    }
}

#[test]
fn open_platform_monthly_quota_is_terminal_and_redacted() {
    let encoded = provider_http_error(
        "lark_open_platform",
        "send_text",
        reqwest::StatusCode::BAD_REQUEST,
        r#"{"code":99991403,"msg":"private reply"}"#,
    );
    let decoded = ChannelProviderError::decode(&encoded).expect("machine provider error");
    assert_eq!(
        decoded.failure_class,
        claw_core::channel_provider_error::ChannelProviderFailureClass::QuotaExhausted
    );
    assert!(!decoded.retryable);
    assert!(!encoded.contains("private reply"));
}

#[tokio::test]
async fn whatsapp_web_scheduled_send_is_blocked_by_default_with_machine_error() {
    let state = AppState::test_default_with_fixture_provider();
    assert!(!state.channels.whatsapp_web_allow_proactive_send);

    let encoded = send_whatsapp_web_bridge_text_message(
        &state,
        "recipient@s.whatsapp.net",
        "scheduled result",
        ChannelDeliverySource::ScheduledTask,
    )
    .await
    .expect_err("experimental proactive send must be opt-in");
    let error = ChannelProviderError::decode(&encoded).expect("machine provider error");
    assert_eq!(
        error.failure_class,
        claw_core::channel_provider_error::ChannelProviderFailureClass::PermissionDenied
    );
    assert_eq!(
        error.provider_error_code.as_deref(),
        Some("proactive_send_disabled")
    );
    assert!(!error.retryable);
}

#[tokio::test]
async fn whatsapp_web_media_uses_the_shared_result_endpoint_and_keeps_receipt_ids() {
    #[derive(Clone, Default)]
    struct BridgeState {
        text_calls: Arc<AtomicUsize>,
        result_calls: Arc<AtomicUsize>,
    }

    async fn send_text(State(state): State<BridgeState>) -> Json<serde_json::Value> {
        state.text_calls.fetch_add(1, Ordering::SeqCst);
        Json(json!({"ok": true, "message_ids": ["text-id"]}))
    }

    async fn send_result(
        State(state): State<BridgeState>,
        Json(payload): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        state.result_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(payload["delivery_source"], "immediate_daemon");
        assert_eq!(payload["schema_version"], 1);
        assert_eq!(payload["text"], "result");
        assert_eq!(payload["media"][0]["kind"], "image");
        assert!(payload["media"][0]["path"]
            .as_str()
            .is_some_and(|path| path.ends_with("image.jpg")));
        Json(json!({"ok": true, "message_ids": ["caption-id", "image-id"]}))
    }

    let bridge_state = BridgeState::default();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind bridge fixture");
    let address = listener.local_addr().expect("bridge fixture address");
    let app = Router::new()
        .route("/v1/send-text", post(send_text))
        .route("/v1/send-result", post(send_result))
        .with_state(bridge_state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve bridge fixture");
    });

    let root = std::env::temp_dir().join(format!(
        "shared-whatsapp-web-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&root).expect("create bridge fixture directory");
    let image = root.join("image.jpg");
    std::fs::write(&image, b"image").expect("write bridge image fixture");
    let mut state = AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.channels.whatsapp_web_enabled = true;
    state.channels.whatsapp_web_bridge_base_url = format!("http://{address}");
    let outcome = send_whatsapp_web_bridge_text_message(
        &state,
        "recipient@s.whatsapp.net",
        &format!("result\nIMAGE_FILE:{}", image.display()),
        ChannelDeliverySource::ImmediateDaemon,
    )
    .await
    .expect("shared bridge delivery");

    assert_eq!(bridge_state.text_calls.load(Ordering::SeqCst), 0);
    assert_eq!(bridge_state.result_calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.provider_message_ids, vec!["caption-id", "image-id"]);
    std::fs::remove_dir_all(root).expect("remove bridge fixture directory");
}

#[tokio::test]
async fn shared_open_platform_sender_delivers_text_image_video_and_audio_for_both_regions() {
    #[derive(Clone, Default)]
    struct MockState {
        image_uploads: Arc<AtomicUsize>,
        file_uploads: Arc<AtomicUsize>,
        message_types: Arc<Mutex<Vec<String>>>,
    }

    async fn mock_token() -> Json<serde_json::Value> {
        Json(json!({
            "tenant_access_token": "tenant-token",
            "expire": 7200
        }))
    }

    async fn mock_image_upload(
        State(state): State<MockState>,
        _body: Bytes,
    ) -> Json<serde_json::Value> {
        state.image_uploads.fetch_add(1, Ordering::SeqCst);
        Json(json!({"code": 0, "data": {"image_key": "image-key"}}))
    }

    async fn mock_file_upload(
        State(state): State<MockState>,
        _body: Bytes,
    ) -> Json<serde_json::Value> {
        state.file_uploads.fetch_add(1, Ordering::SeqCst);
        Json(json!({"code": 0, "data": {"file_key": "file-key"}}))
    }

    async fn mock_message(
        State(state): State<MockState>,
        Json(payload): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        state
            .message_types
            .lock()
            .expect("message types")
            .push(payload["msg_type"].as_str().unwrap_or_default().to_string());
        Json(json!({"code": 0, "data": {"message_id": "om-shared-fixture"}}))
    }

    for (channel_tag, adapter) in [
        (
            "feishu",
            claw_core::channel_capabilities::ChannelAdapterKind::FeishuOpenPlatform,
        ),
        (
            "lark",
            claw_core::channel_capabilities::ChannelAdapterKind::LarkOpenPlatform,
        ),
    ] {
        let mock_state = MockState::default();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind open platform fixture");
        let address = listener.local_addr().expect("fixture address");
        let app = Router::new()
            .route(
                "/open-apis/auth/v3/tenant_access_token/internal",
                post(mock_token),
            )
            .route("/open-apis/im/v1/images", post(mock_image_upload))
            .route("/open-apis/im/v1/files", post(mock_file_upload))
            .route("/open-apis/im/v1/messages", post(mock_message))
            .with_state(mock_state.clone());
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve open platform fixture");
        });

        let root = std::env::temp_dir().join(format!(
            "shared-open-platform-{channel_tag}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).expect("create fixture directory");
        let image = root.join("image.jpg");
        let video = root.join("video.mp4");
        let audio = root.join("voice.opus");
        std::fs::write(&image, b"image").expect("write image fixture");
        std::fs::write(&video, b"video").expect("write video fixture");
        std::fs::write(&audio, b"audio").expect("write audio fixture");
        let answer = format!(
            "download complete\nIMAGE_FILE:{}\nVIDEO_FILE:{}\nVOICE_FILE:{}",
            image.display(),
            video.display(),
            audio.display()
        );
        let mut state = AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = root.clone();
        let outcome = send_feishu_lark_answer(
            &state,
            channel_tag,
            adapter,
            &format!("http://{address}"),
            "app-id",
            "app-secret",
            "chat-id",
            &answer,
        )
        .await
        .expect("shared open platform delivery");

        assert_eq!(mock_state.image_uploads.load(Ordering::SeqCst), 1);
        assert_eq!(mock_state.file_uploads.load(Ordering::SeqCst), 2);
        assert_eq!(outcome.provider_message_ids.len(), 4);
        assert_eq!(
            *mock_state.message_types.lock().expect("message types"),
            vec!["text", "image", "media", "audio"]
        );
        std::fs::remove_dir_all(root).expect("remove fixture directory");
    }
}

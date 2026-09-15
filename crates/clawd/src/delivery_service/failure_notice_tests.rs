use super::*;
use axum::{extract::State, routing::post, Json, Router};
use std::sync::{Arc, Mutex};

struct TempFiles(std::path::PathBuf);
impl TempFiles {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("delivery-notice-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn file(&self, name: &str) -> std::path::PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, b"fixture").unwrap();
        path
    }
}
impl Drop for TempFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn task() -> (ClaimedTask, Value) {
    let payload = json!({"channel":"whatsapp", "adapter":"whatsapp_web",
        "channel_ingress":{"locale":"zh-CN", "adapter":"whatsapp_web",
            "reply_target":{"kind":"chat", "external_id":"fixture-recipient"}}});
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: uuid::Uuid::new_v4().to_string(),
        user_id: 1,
        chat_id: 9,
        user_key: None,
        channel: "whatsapp".into(),
        external_user_id: None,
        external_chat_id: Some("fixture-recipient".into()),
        kind: "ask".into(),
        payload_json: payload.to_string(),
    };
    (task, payload)
}

#[test]
fn local_preflight_codes_are_not_lost_or_inferred_from_prose() {
    for prefix in [
        "channel_media_preflight_failed",
        "whatsapp_cloud_media_preflight_failed",
    ] {
        let raw = format!("{prefix}:channel_media_too_large:127140539:104857600");
        let error = decode_local_media_error(&raw).unwrap();
        assert_eq!(
            error.provider_error_code.as_deref(),
            Some("channel_media_too_large")
        );
        assert_eq!(
            error.failure_class,
            ChannelProviderFailureClass::PayloadRejected
        );
        assert!(!error.retryable);
        assert_eq!(
            super::super::delivery_failure_fields(&raw).3,
            error.provider_error_code
        );
    }
    for invalid in [
        "file is too large",
        "channel_media_preflight_failed:secret:2:1",
        "channel_media_preflight_failed:channel_media_too_large:2:1:private",
        "channel_media_preflight_failed:channel_media_too_large:abc:1",
    ] {
        assert!(decode_local_media_error(invalid).is_none());
    }
}

#[test]
fn notice_uses_verified_paths_and_cannot_resend_attachments() {
    let root = TempFiles::new();
    let video = root.file("clip.mp4");
    let state = AppState::test_default_with_fixture_provider();
    let (task, payload) = task();
    let envelope = super::super::build_scheduled_delivery_envelope(
        &state,
        &task,
        &payload,
        &format!("VIDEO_FILE:{}", video.display()),
    )
    .unwrap();
    let files = file_locations(&envelope, &root.0);
    assert_eq!(files.len(), 1);
    assert!(files[0].exists);
    assert_eq!(
        files[0].directory,
        root.0.canonicalize().unwrap().to_string_lossy()
    );
    assert_eq!(files[0].size_bytes, Some(7));
    let text = format!("视频投送失败，文件保留在主机的 `{}`。", files[0].path);
    assert_eq!(
        notice_text(&json!({"text":text}).to_string(), &files, &root.0).unwrap(),
        text
    );
    for invalid in [
        json!({"text":"failed"}),
        json!({"text":""}),
        json!({"text":text,"action":"send"}),
        json!({"text":format!("FILE:{}", files[0].path)}),
    ] {
        assert!(notice_text(&invalid.to_string(), &files, &root.0).is_err());
    }
}

#[derive(Clone, Default)]
struct MockState {
    prompts: Arc<Mutex<Vec<String>>>,
    sends: Arc<Mutex<Vec<Value>>>,
    model_text: Arc<Mutex<String>>,
    reject_model: Arc<std::sync::atomic::AtomicBool>,
}

async fn model(
    State(mock): State<MockState>,
    Json(body): Json<Value>,
) -> (axum::http::StatusCode, Json<Value>) {
    mock.prompts.lock().unwrap().push(body.to_string());
    if mock.reject_model.load(std::sync::atomic::Ordering::SeqCst) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error":{"code":"invalid_request"}})),
        );
    }
    let text = mock.model_text.lock().unwrap().clone();
    (
        axum::http::StatusCode::OK,
        Json(json!({"choices":[{"message":{"role":"assistant", "content":
        json!({"text":text}).to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":8,"total_tokens":20}})),
    )
}

async fn send_result(
    State(mock): State<MockState>,
    Json(body): Json<Value>,
) -> (axum::http::StatusCode, Json<Value>) {
    mock.sends.lock().unwrap().push(body.clone());
    if body["media"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"schema_version":1,
            "error_code":"channel_media_too_large", "message_key":"channel.media.preflight.too_large",
            "retryable":false,"actual_bytes":127140539,"max_bytes":104857600})),
        )
    } else {
        (
            axum::http::StatusCode::OK,
            Json(json!({"message_ids":["notice-message"]})),
        )
    }
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn test_state(mock: MockState) -> (AppState, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/v1/chat/completions", post(model))
        .route("/chat/completions", post(model))
        .route("/v1/send-result", post(send_result))
        .route("/v1/send-text", post(send_result))
        .with_state(mock);
    let server = Server(tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    }));
    let mut state = AppState::test_default_with_fixture_provider()
        .with_seeded_db_schema()
        .with_prompt_layers_installed();
    let provider = Arc::make_mut(&mut state.core.llm_providers[0]);
    provider.config.provider_type = "openai_compat".into();
    provider.config.name = "fixture-notice-model".into();
    provider.config.base_url = base.clone();
    provider.client = reqwest::Client::builder().no_proxy().build().unwrap();
    state.core.active_provider_type = Some("openai_compat".into());
    state.core.http_client = reqwest::Client::builder().no_proxy().build().unwrap();
    state.channels.whatsapp_web_enabled = true;
    state.channels.whatsapp_web_bridge_base_url = base;
    (state, server)
}

#[tokio::test]
async fn oversize_delivery_reaches_model_then_sends_one_text_notice_without_replaying_media() {
    let root = TempFiles::new();
    let video = root.file("clip.mp4").canonicalize().unwrap();
    let expected = format!(
        "视频超过发送限制，没有发出；文件保存在主机 `{}`。",
        video.display()
    );
    let mock = MockState::default();
    *mock.model_text.lock().unwrap() = expected.clone();
    let (state, _server) = test_state(mock.clone()).await;
    let (task, payload) = task();
    let mut envelope = super::super::build_scheduled_delivery_envelope(
        &state,
        &task,
        &payload,
        &format!("VIDEO_FILE:{}", video.display()),
    )
    .unwrap();
    envelope.source = ChannelDeliverySource::ImmediateDaemon;
    for _ in 0..2 {
        let result = super::super::deliver_task_envelope(&state, &task, &payload, &envelope)
            .await
            .unwrap();
        assert_eq!(result.status, ChannelDeliveryServiceStatus::Failed);
        assert_eq!(
            result.receipt.unwrap().provider_error_code.as_deref(),
            Some("channel_media_too_large")
        );
    }
    let prompts = mock.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("channel_media_too_large"));
    assert!(prompts[0].contains(video.to_str().unwrap()));
    assert!(prompts[0].contains("zh-CN"));
    let sends = mock.sends.lock().unwrap();
    assert_eq!(sends.len(), 2);
    assert_eq!(sends[0]["media"].as_array().unwrap().len(), 1);
    assert!(sends[1].get("media").is_none());
    assert_eq!(sends[1]["text"], expected);
    assert_eq!(sends[0]["to"], sends[1]["to"]);
    assert_eq!(sends[1]["delivery_source"], "immediate_daemon");
}

#[tokio::test]
async fn model_unavailable_keeps_original_receipt_and_can_retry_only_the_explanation() {
    use std::sync::atomic::Ordering;
    let root = TempFiles::new();
    let file = root.file("notes.txt").canonicalize().unwrap();
    let mock = MockState::default();
    mock.reject_model.store(true, Ordering::SeqCst);
    *mock.model_text.lock().unwrap() = format!(
        "Sending failed. The host file remains at `{}`.",
        file.display()
    );
    let (state, _server) = test_state(mock.clone()).await;
    let (task, payload) = task();
    let mut envelope = super::super::build_scheduled_delivery_envelope(
        &state,
        &task,
        &payload,
        &format!("FILE:{}", file.display()),
    )
    .unwrap();
    envelope.source = ChannelDeliverySource::ImmediateDaemon;
    let first = super::super::deliver_task_envelope(&state, &task, &payload, &envelope).await;
    assert!(first
        .unwrap_err()
        .to_string()
        .contains("delivery_failure_notice_model_unavailable"));
    assert_eq!(mock.sends.lock().unwrap().len(), 1);
    mock.reject_model.store(false, Ordering::SeqCst);
    let second = super::super::deliver_task_envelope(&state, &task, &payload, &envelope)
        .await
        .unwrap();
    assert_eq!(second.status, ChannelDeliveryServiceStatus::Failed);
    let sends = mock.sends.lock().unwrap();
    assert_eq!(sends.len(), 2);
    assert!(sends[1].get("media").is_none());
}

#[test]
fn partial_receipts_trigger_notice_but_proactive_success_or_retryable_receipts_do_not() {
    let state = AppState::test_default_with_fixture_provider();
    let (task, payload) = task();
    let mut envelope =
        super::super::build_scheduled_delivery_envelope(&state, &task, &payload, "result").unwrap();
    let mut receipt = super::super::accepted_delivery_receipt(&envelope, Default::default(), 1);
    receipt.status = ChannelDeliveryStatus::Partial;
    let mut result = super::super::result_from_existing_receipt(receipt);
    assert!(should_notify(&envelope, &result));
    result.retryable = true;
    assert!(!should_notify(&envelope, &result));
    result.retryable = false;
    envelope.source = ChannelDeliverySource::ProactiveNotice;
    assert!(!should_notify(&envelope, &result));
    envelope.source = ChannelDeliverySource::ScheduledTask;
    result.status = ChannelDeliveryServiceStatus::Accepted;
    assert!(!should_notify(&envelope, &result));
}

use super::{
    build_feishu_long_connection_config, extract_bind_key_candidate,
    extract_pending_bind_token_candidate, feishu_delivery_error_text, feishu_provider_http_error,
    handle_incoming_feishu_text, install_tls_crypto_provider, is_unbound_allowed_command,
    parse_im_text_from_event_body, AppState, FeishuConfig, FeishuSection,
    FEISHU_BIND_REQUIRED_FALLBACK, FEISHU_I18N_BIND_REQUIRED_KEY,
};
use crate::config_helpers::feishu_t;
use crate::media_helpers::feishu_media_agent_context;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use claw_core::channel_open_platform::OpenPlatformTokenCache;
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[test]
fn tls_crypto_provider_installation_is_idempotent() {
    install_tls_crypto_provider().expect("initial provider installation");
    install_tls_crypto_provider().expect("repeated provider installation");
    assert!(rustls::crypto::CryptoProvider::get_default().is_some());
}

#[test]
fn feishu_long_connection_uses_its_configured_api_base() {
    let section = FeishuSection {
        app_id: "fixture-app".to_string(),
        app_secret: "fixture-secret".to_string(),
        api_base_url: "https://feishu.example.test/".to_string(),
        ..FeishuSection::default()
    };
    let sdk = build_feishu_long_connection_config(&section);
    assert_eq!(sdk.base_url, "https://feishu.example.test");
}

#[test]
fn feishu_provider_code_overrides_legacy_http_status_classification() {
    let encoded =
        feishu_provider_http_error("send_text", 400, r#"{"code":230020,"msg":"private prose"}"#);
    let error = claw_core::channel_provider_error::ChannelProviderError::decode(&encoded)
        .expect("typed provider error");
    assert_eq!(
        error.failure_class,
        claw_core::channel_provider_error::ChannelProviderFailureClass::RateLimited
    );
    assert_eq!(error.provider_error_code.as_deref(), Some("230020"));
    assert!(error.retryable);
    assert!(!encoded.contains("private prose"));
}

#[test]
fn feishu_delivery_errors_resolve_to_localized_copy_without_machine_payloads() {
    let config = FeishuConfig {
        feishu: FeishuSection {
            language: "zh-CN".to_string(),
            ..FeishuSection::default()
        },
    };
    let encoded = feishu_provider_http_error(
        "send_text",
        400,
        r#"{"code":99991403,"msg":"private prose"}"#,
    );
    let localized = feishu_delivery_error_text(&config, &encoded);
    assert!(localized.contains("额度"));
    assert!(!localized.contains("private prose"));
    assert!(!localized.contains("__CHANNEL_PROVIDER_ERROR"));

    assert_eq!(
        feishu_delivery_error_text(&config, "unstructured internal detail"),
        claw_core::channel_i18n::safe_generic_text_for_locale("zh-CN")
    );
}

#[test]
fn unbound_plain_text_requires_binding_prompt() {
    assert!(!is_unbound_allowed_command("hello"));
    assert_eq!(extract_bind_key_candidate("hello", false), None);
}

#[test]
fn missing_feishu_i18n_uses_safe_localized_fallback() {
    let config = FeishuConfig {
        feishu: FeishuSection {
            language: "zh-CN".to_string(),
            i18n_path: "/tmp/agent-runtime-no-such-feishud.zh-CN.toml".to_string(),
            ..FeishuSection::default()
        },
    };

    let text = feishu_t(
        &config,
        FEISHU_I18N_BIND_REQUIRED_KEY,
        FEISHU_BIND_REQUIRED_FALLBACK,
    );
    assert_eq!(
        text,
        claw_core::channel_i18n::safe_generic_text_for_locale("zh-CN")
    );
    assert!(!text.contains("message_key="));
}

#[test]
fn unbound_key_command_keeps_binding_flow_available() {
    assert_eq!(
        extract_bind_key_candidate("/key rk_live_123", false).as_deref(),
        Some("rk_live_123")
    );
}

#[test]
fn unbound_help_and_start_are_allowed() {
    assert!(is_unbound_allowed_command("/start"));
    assert!(is_unbound_allowed_command("/help"));
    assert!(!is_unbound_allowed_command("/start/docs"));
    assert!(!is_unbound_allowed_command("/help.md"));
}

#[test]
fn waiting_key_state_accepts_plain_key_reply() {
    assert_eq!(
        extract_bind_key_candidate("rk_live_abc", true).as_deref(),
        Some("rk_live_abc")
    );
}

#[test]
fn waiting_key_state_rejects_non_binding_commands() {
    assert_eq!(
        extract_bind_key_candidate("/run image_vision {}", true),
        None
    );
    assert_eq!(extract_bind_key_candidate("/crypto btc", true), None);
}

#[test]
fn unbound_media_like_empty_text_requires_binding_prompt() {
    assert!(!is_unbound_allowed_command(""));
    assert_eq!(extract_bind_key_candidate("", false), None);
}

#[test]
fn feishu_media_agent_context_uses_machine_fields() {
    let text = feishu_media_agent_context("image", "data/feishud/image/chat/file.jpg");
    let value: Value = serde_json::from_str(&text).expect("media context json");
    assert_eq!(value["event_type"], "channel_media_saved");
    assert_eq!(value["channel"], "feishu");
    assert_eq!(value["media_kind"], "image");
    assert_eq!(value["source_message_type"], "image");
    assert_eq!(
        value["workspace_relative_path"],
        "data/feishud/image/chat/file.jpg"
    );
    assert_eq!(value["locator"]["kind"], "workspace_relative_path");
    assert_eq!(value["locator"]["path"], "data/feishud/image/chat/file.jpg");
}

#[test]
fn extract_pending_bind_token_from_start_or_plain_text() {
    assert_eq!(
        extract_pending_bind_token_candidate("/start pb-test-token").as_deref(),
        Some("pb-test-token")
    );
    assert_eq!(
        extract_pending_bind_token_candidate("pb-test-token").as_deref(),
        Some("pb-test-token")
    );
    assert_eq!(extract_pending_bind_token_candidate("/start"), None);
}

#[test]
fn text_event_keeps_platform_message_id() {
    let event = json!({
        "header": {"event_type": "im.message.receive_v1"},
        "event": {
            "sender": {"sender_id": {"open_id": "open-1"}},
            "message": {
                "message_id": "message-1",
                "message_type": "text",
                "chat_id": "chat-1",
                "content": "{\"text\":\"hello\"}"
            }
        }
    });

    let parsed = parse_im_text_from_event_body(&event).expect("parse feishu text event");
    assert_eq!(parsed.0, "open-1");
    assert_eq!(parsed.1, "chat-1");
    assert_eq!(parsed.2, "message-1");
    assert_eq!(parsed.3, "hello");
}

#[test]
fn start_without_token_does_not_create_pending_bind_token() {
    assert_eq!(extract_pending_bind_token_candidate("/start"), None);
    assert_eq!(extract_pending_bind_token_candidate("/start   "), None);
}

#[tokio::test]
async fn feishu_pending_bind_requires_explicit_token() {
    #[derive(Clone)]
    struct MockClawdState {
        detect_calls: Arc<AtomicUsize>,
        bind_calls: Arc<AtomicUsize>,
    }

    async fn mock_resolve() -> Json<Value> {
        Json(json!({
            "ok": true,
            "data": { "bound": false, "identity": null },
            "error": null
        }))
    }

    async fn mock_detect(
        State(state): State<MockClawdState>,
        Json(payload): Json<Value>,
    ) -> Json<Value> {
        state.detect_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(payload["bind_token"], "pb-test-token");
        assert_eq!(payload["external_user_id"], "ou_test_pending_bind");
        assert_eq!(payload["external_chat_id"], "oc_test_pending_bind");
        Json(json!({
            "ok": true,
            "data": {
                "matched": true,
                "session": {
                    "session_id": 7,
                    "channel": "feishu",
                    "bind_token": "pb-test-token",
                    "status": "bound",
                    "external_user_id": "ou_test_pending_bind",
                    "external_chat_id": "oc_test_pending_bind",
                    "error_text": null,
                    "created_at": "100",
                    "updated_at": "101",
                    "expires_at": "9999999999",
                    "entry_url": "https://applink.feishu.cn/client/bot/open?appId=cli_test_feishu_app"
                }
            },
            "error": null
        }))
    }

    async fn mock_bind(
        State(state): State<MockClawdState>,
        Json(_payload): Json<Value>,
    ) -> Json<Value> {
        state.bind_calls.fetch_add(1, Ordering::SeqCst);
        Json(json!({
            "ok": true,
            "data": {
                "user_key": "rk-should-not-bind",
                "role": "user",
                "user_id": 1,
                "chat_id": 1
            },
            "error": null
        }))
    }

    let clawd_state = MockClawdState {
        detect_calls: Arc::new(AtomicUsize::new(0)),
        bind_calls: Arc::new(AtomicUsize::new(0)),
    };
    let clawd_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock clawd");
    let clawd_addr = clawd_listener.local_addr().expect("mock clawd addr");
    let clawd_app = Router::new()
        .route("/v1/auth/channel/resolve", post(mock_resolve))
        .route("/v1/auth/channel-binds/feishu/detect", post(mock_detect))
        .route("/v1/auth/channel/bind", post(mock_bind))
        .with_state(clawd_state.clone());
    tokio::spawn(async move {
        axum::serve(clawd_listener, clawd_app)
            .await
            .expect("serve mock clawd");
    });

    #[derive(Clone)]
    struct MockFeishuState {
        sent_texts: Arc<Mutex<Vec<String>>>,
    }

    async fn mock_token() -> Json<Value> {
        Json(json!({
            "tenant_access_token": "tenant_token",
            "expire": 7200
        }))
    }

    async fn mock_send(
        State(state): State<MockFeishuState>,
        Json(payload): Json<Value>,
    ) -> Json<Value> {
        let content_str = payload["content"].as_str().expect("content string");
        let content: Value = serde_json::from_str(content_str).expect("content json");
        let text = content["text"].as_str().expect("text").to_string();
        state.sent_texts.lock().expect("sent texts").push(text);
        Json(json!({ "code": 0, "data": { "message_id": "om-feishu-bind" } }))
    }

    let feishu_state = MockFeishuState {
        sent_texts: Arc::new(Mutex::new(Vec::new())),
    };
    let i18n_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/i18n/feishud.zh-CN.toml")
        .to_string_lossy()
        .to_string();
    let feishu_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock feishu");
    let feishu_addr = feishu_listener.local_addr().expect("mock feishu addr");
    let feishu_app = Router::new()
        .route(
            "/open-apis/auth/v3/tenant_access_token/internal",
            post(mock_token),
        )
        .route("/open-apis/im/v1/messages", post(mock_send))
        .with_state(feishu_state.clone());
    tokio::spawn(async move {
        axum::serve(feishu_listener, feishu_app)
            .await
            .expect("serve mock feishu");
    });

    let state = AppState {
        config: FeishuConfig {
            feishu: FeishuSection {
                enabled: true,
                clawd_base_url: format!("http://{clawd_addr}"),
                api_base_url: format!("http://{feishu_addr}"),
                app_id: "cli_test_feishu_app".to_string(),
                app_secret: "cli_test_secret".to_string(),
                i18n_path,
                ..FeishuSection::default()
            },
        },
        client: Client::new(),
        token_cache: Arc::new(OpenPlatformTokenCache::default()),
        workspace_root: std::env::temp_dir(),
        pending_key_bind_by_chat: Arc::new(Mutex::new(std::collections::HashSet::new())),
    };

    handle_incoming_feishu_text(
        state,
        "ou_test_pending_bind".to_string(),
        "oc_test_pending_bind".to_string(),
        "om_test_pending_bind".to_string(),
        "/start pb-test-token".to_string(),
        false,
    )
    .await;

    assert_eq!(clawd_state.detect_calls.load(Ordering::SeqCst), 1);
    assert_eq!(clawd_state.bind_calls.load(Ordering::SeqCst), 0);
    let sent = feishu_state.sent_texts.lock().expect("sent texts");
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("绑定成功") || sent[0].contains("bound"));
}

use super::{
    build_lark_long_connection_config, build_lark_submit_request, extract_bind_key_candidate,
    extract_pending_bind_token_candidate, install_tls_crypto_provider, is_unbound_allowed_command,
    lark_delivery_error_text, lark_provider_http_error, parse_im_text_from_event_body, LarkConfig,
    LarkSection, LARK_BIND_REQUIRED_FALLBACK, LARK_I18N_BIND_REQUIRED_KEY,
};
use crate::config_helpers::lark_t;
use serde_json::json;

#[test]
fn tls_crypto_provider_installation_is_idempotent() {
    install_tls_crypto_provider().expect("initial provider installation");
    install_tls_crypto_provider().expect("repeated provider installation");
    assert!(rustls::crypto::CryptoProvider::get_default().is_some());
}

#[test]
fn lark_long_connection_uses_its_configured_api_base() {
    let section = LarkSection {
        app_id: "fixture-app".to_string(),
        app_secret: "fixture-secret".to_string(),
        api_base_url: "https://lark.example.test/".to_string(),
        ..LarkSection::default()
    };
    let sdk = build_lark_long_connection_config(&section);
    assert_eq!(sdk.base_url, "https://lark.example.test");
}

#[test]
fn lark_provider_code_overrides_legacy_http_status_classification() {
    let encoded =
        lark_provider_http_error("send_text", 400, r#"{"code":230020,"msg":"private prose"}"#);
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
fn lark_delivery_errors_resolve_to_localized_copy_without_machine_payloads() {
    let config = LarkConfig {
        lark: LarkSection {
            language: "en-US".to_string(),
            ..LarkSection::default()
        },
    };
    let encoded = lark_provider_http_error(
        "send_text",
        400,
        r#"{"code":99991403,"msg":"private prose"}"#,
    );
    let localized = lark_delivery_error_text(&config, &encoded);
    assert!(localized.contains("quota"));
    assert!(!localized.contains("private prose"));
    assert!(!localized.contains("__CHANNEL_PROVIDER_ERROR"));

    assert_eq!(
        lark_delivery_error_text(&config, "unstructured internal detail"),
        claw_core::channel_i18n::safe_generic_text_for_locale("en-US")
    );
}

#[test]
fn unbound_plain_text_requires_binding_prompt() {
    assert!(!is_unbound_allowed_command("hello"));
    assert_eq!(extract_bind_key_candidate("hello", false), None);
}

#[test]
fn missing_lark_i18n_uses_safe_localized_fallback() {
    let config = LarkConfig {
        lark: LarkSection {
            language: "en-US".to_string(),
            i18n_path: "/tmp/agent-runtime-no-such-larkd.en-US.toml".to_string(),
            ..LarkSection::default()
        },
    };

    let text = lark_t(
        &config,
        LARK_I18N_BIND_REQUIRED_KEY,
        LARK_BIND_REQUIRED_FALLBACK,
    );
    assert_eq!(
        text,
        claw_core::channel_i18n::safe_generic_text_for_locale("en-US")
    );
    assert!(!text.contains("message_key="));
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

    let parsed = parse_im_text_from_event_body(&event).expect("parse lark text event");
    assert_eq!(parsed.0, "open-1");
    assert_eq!(parsed.1, "chat-1");
    assert_eq!(parsed.2, "message-1");
    assert_eq!(parsed.3, "hello");
}

#[test]
fn unbound_key_command_keeps_binding_flow_available() {
    assert_eq!(
        extract_bind_key_candidate("/key rk_live_123", false).as_deref(),
        Some("rk_live_123")
    );
}

#[test]
fn official_setup_bind_token_is_detected_without_treating_it_as_an_auth_key() {
    assert_eq!(
        extract_pending_bind_token_candidate("pb-abc123").as_deref(),
        Some("pb-abc123")
    );
    assert_eq!(
        extract_pending_bind_token_candidate("/start pb-abc123").as_deref(),
        Some("pb-abc123")
    );
    assert_eq!(extract_pending_bind_token_candidate("/start"), None);
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
fn lark_media_is_an_ask_attachment_without_synthetic_instruction() {
    let attachment = claw_core::channel_ingress::ChannelIngressAttachment {
        kind: "video".to_string(),
        path: "data/larkd/video/chat/file.mp4".to_string(),
        mime_type: Some("video/mp4".to_string()),
        size: Some(84),
    };
    let request = build_lark_submit_request(
        "en-US",
        "open-1",
        "chat-1",
        "message-1",
        String::new(),
        vec![attachment.clone()],
        Some("rk-test".to_string()),
    );
    assert!(matches!(request.kind, claw_core::types::TaskKind::Ask));
    assert_eq!(request.payload["text"], "");
    assert_eq!(request.payload["attachments"][0]["path"], attachment.path);
    let ingress = request.ingress.expect("ingress");
    assert_eq!(ingress.attachments, vec![attachment]);
}

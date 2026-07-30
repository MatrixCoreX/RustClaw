use super::{
    extract_bind_key_candidate, extract_pending_bind_token_candidate, install_tls_crypto_provider,
    is_unbound_allowed_command, lark_media_agent_context,
};
use serde_json::Value;

#[test]
fn tls_crypto_provider_installation_is_idempotent() {
    install_tls_crypto_provider().expect("initial provider installation");
    install_tls_crypto_provider().expect("repeated provider installation");
    assert!(rustls::crypto::CryptoProvider::get_default().is_some());
}

#[test]
fn unbound_plain_text_requires_binding_prompt() {
    assert!(!is_unbound_allowed_command("hello"));
    assert_eq!(extract_bind_key_candidate("hello", false), None);
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
fn lark_media_agent_context_uses_machine_fields() {
    let text = lark_media_agent_context("media", "data/larkd/video/chat/file.mp4");
    let value: Value = serde_json::from_str(&text).expect("media context json");
    assert_eq!(value["event_type"], "channel_media_saved");
    assert_eq!(value["channel"], "lark");
    assert_eq!(value["media_kind"], "video");
    assert_eq!(value["source_message_type"], "media");
    assert_eq!(
        value["workspace_relative_path"],
        "data/larkd/video/chat/file.mp4"
    );
    assert_eq!(value["locator"]["kind"], "workspace_relative_path");
    assert_eq!(value["locator"]["path"], "data/larkd/video/chat/file.mp4");
}

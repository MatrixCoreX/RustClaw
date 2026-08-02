use super::{
    extract_bind_key_candidate, is_unbound_allowed_command, whatsapp_typing_indicator_payload,
    WA_BIND_REQUIRED_FALLBACK, WA_I18N_BIND_REQUIRED_KEY,
};
use claw_core::channel_commands::ChannelCommandCatalog;
use claw_core::channel_i18n::text_from_path;
use std::path::Path;

#[test]
fn whatsapp_cloud_media_specs_reject_unsupported_formats_and_oversize_files() {
    use claw_core::channel_media_limits::{
        validate_local_media_file, whatsapp_cloud_upload_spec, WhatsappCloudMediaKind,
    };

    let root = std::env::temp_dir().join(format!("whatsapp-media-limit-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create media limit dir");
    let video = root.join("oversized.mp4");
    let (mime, max_bytes, label) =
        whatsapp_cloud_upload_spec(&video, WhatsappCloudMediaKind::Video).expect("video spec");
    let file = std::fs::File::create(&video).expect("create sparse video");
    file.set_len(max_bytes + 1)
        .expect("set sparse video length");
    assert_eq!(mime, "video/mp4");
    assert_eq!(
        validate_local_media_file(&video, "WhatsApp Cloud", label, max_bytes).unwrap_err(),
        format!(
            "channel_media_preflight_failed:channel_media_too_large:{}:{}",
            max_bytes + 1,
            max_bytes
        )
    );
    assert!(
        whatsapp_cloud_upload_spec(Path::new("clip.webm"), WhatsappCloudMediaKind::Video)
            .unwrap_err()
            .starts_with("whatsapp_cloud_video_format_unsupported:")
    );
    std::fs::remove_dir_all(root).expect("remove media limit dir");
}

fn default_catalog() -> ChannelCommandCatalog {
    ChannelCommandCatalog::default()
}

fn unbound_allowed(text: &str) -> bool {
    is_unbound_allowed_command(&default_catalog(), "whatsapp", text)
}

#[test]
fn unbound_plain_text_requires_binding_prompt() {
    assert!(!unbound_allowed("hello"));
    assert_eq!(extract_bind_key_candidate("hello", false), None);
}

#[test]
fn queued_task_feedback_uses_native_typing_payload_not_chat_text() {
    let payload = whatsapp_typing_indicator_payload("wamid.inbound-1");

    assert_eq!(payload["messaging_product"], "whatsapp");
    assert_eq!(payload["status"], "read");
    assert_eq!(payload["message_id"], "wamid.inbound-1");
    assert_eq!(payload["typing_indicator"]["type"], "text");
    assert!(payload.get("to").is_none());
    assert!(payload.get("text").is_none());
}

#[test]
fn unbound_key_command_keeps_binding_flow_available() {
    assert_eq!(
        extract_bind_key_candidate("/key rk_live_123", false).as_deref(),
        Some("rk_live_123")
    );
}

#[test]
fn unbound_start_and_help_are_allowed_without_task_submission() {
    assert!(unbound_allowed("/start"));
    assert!(unbound_allowed("/help"));
}

#[test]
fn waiting_key_state_accepts_plain_key_reply() {
    assert_eq!(
        extract_bind_key_candidate("rk_live_abc", true).as_deref(),
        Some("rk_live_abc")
    );
}

#[test]
fn waiting_key_state_does_not_treat_business_commands_as_key() {
    assert_eq!(extract_bind_key_candidate("/run weather {}", true), None);
    assert_eq!(extract_bind_key_candidate("/crypto btc", true), None);
}

#[test]
fn unbound_media_like_empty_text_requires_binding_prompt() {
    assert!(!unbound_allowed(""));
    assert_eq!(extract_bind_key_candidate("", false), None);
}

#[test]
fn whatsapp_i18n_is_locale_specific_with_safe_fallback() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let zh_path = root.join("configs/i18n/whatsapp-cloud.zh-CN.toml");
    let en_path = root.join("configs/i18n/whatsapp-cloud.en-US.toml");
    let zh = text_from_path(
        zh_path.to_string_lossy().as_ref(),
        WA_I18N_BIND_REQUIRED_KEY,
        "fallback",
    );
    let en = text_from_path(
        en_path.to_string_lossy().as_ref(),
        WA_I18N_BIND_REQUIRED_KEY,
        "fallback",
    );

    assert!(zh.contains("请先发送"));
    assert!(!zh.contains("Please send"));
    assert!(en.contains("Please send"));
    assert!(!en.contains("请先"));
    let missing = text_from_path(
        "/tmp/agent-runtime-no-such-whatsapp-cloud.zh-CN.toml",
        WA_I18N_BIND_REQUIRED_KEY,
        WA_BIND_REQUIRED_FALLBACK,
    );
    assert_eq!(
        missing,
        claw_core::channel_i18n::safe_generic_text_for_locale("zh-CN")
    );
    assert!(!missing.contains("message_key="));
}

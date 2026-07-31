use super::{
    extract_bind_key_candidate, is_unbound_allowed_command, WA_BIND_REQUIRED_FALLBACK,
    WA_I18N_BIND_REQUIRED_KEY,
};
use claw_core::channel_commands::ChannelCommandCatalog;
use claw_core::channel_delivery_tokens::{
    legacy_delivery_tokens, strip_legacy_local_delivery_lines, LegacyDeliveryKind,
};
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
    assert!(
        validate_local_media_file(&video, "WhatsApp Cloud", label, max_bytes)
            .unwrap_err()
            .contains("16 MiB")
    );
    assert!(
        whatsapp_cloud_upload_spec(Path::new("clip.webm"), WhatsappCloudMediaKind::Video)
            .unwrap_err()
            .contains("H.264")
    );
    std::fs::remove_dir_all(root).expect("remove media limit dir");
}

#[test]
fn outbound_media_tokens_preserve_text_and_extract_image_and_video() {
    let root = std::env::temp_dir().join(format!(
        "whatsapp-outbound-media-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create fixture dir");
    let image = root.join("image.jpg");
    let video = root.join("video.mp4");
    std::fs::write(&image, b"image").expect("write image fixture");
    std::fs::write(&video, b"video").expect("write video fixture");
    let answer = format!(
        "download complete\nIMAGE_FILE:{}\nVIDEO_FILE:{}",
        image.display(),
        video.display()
    );

    assert_eq!(
        strip_legacy_local_delivery_lines(&answer),
        "download complete"
    );
    let tokens = legacy_delivery_tokens(&answer);
    assert_eq!(tokens[0].kind, LegacyDeliveryKind::Image);
    assert_eq!(tokens[0].reference, image.to_string_lossy());
    assert_eq!(tokens[1].kind, LegacyDeliveryKind::Video);
    assert_eq!(tokens[1].reference, video.to_string_lossy());
    std::fs::remove_dir_all(root).expect("remove fixture dir");
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

use super::*;

fn test_catalog() -> TextCatalog {
    TextCatalog {
        current: HashMap::from([
            (
                "telegram.msg.delivery_media_reason_unreadable".to_string(),
                "unreadable".to_string(),
            ),
            (
                "telegram.msg.delivery_media_reason_not_regular_file".to_string(),
                "not a file".to_string(),
            ),
            (
                "telegram.msg.delivery_media_reason_empty".to_string(),
                "empty".to_string(),
            ),
            (
                "telegram.msg.delivery_media_reason_too_large".to_string(),
                "{actual_mib} over {max_mib}".to_string(),
            ),
            (
                "telegram.msg.delivery_media_reason_provider_failed".to_string(),
                "provider failed".to_string(),
            ),
            (
                "telegram.msg.delivery_media_failed_ui_fallback".to_string(),
                "{filename}: {reason}; open UI".to_string(),
            ),
            (
                "telegram.msg.delivery_media_failed_retry".to_string(),
                "{filename}: {reason}; retry".to_string(),
            ),
        ]),
        safe_fallback: "fallback".to_string(),
    }
}

#[test]
fn image_uses_photo_with_document_fallback_until_photo_limit() {
    assert_eq!(
        telegram_upload_method(TelegramMediaKind::Image, 1),
        TelegramUploadMethod::Photo
    );
    assert_eq!(
        telegram_upload_method(
            TelegramMediaKind::Image,
            claw_core::channel_media_limits::telegram_image_max_bytes() + 1,
        ),
        TelegramUploadMethod::Document
    );
    assert_eq!(
        telegram_upload_method(TelegramMediaKind::Video, 1),
        TelegramUploadMethod::Video
    );
    assert_eq!(
        telegram_upload_method(TelegramMediaKind::Voice, 1),
        TelegramUploadMethod::Voice
    );
    assert_eq!(
        telegram_upload_method(TelegramMediaKind::Audio, 1),
        TelegramUploadMethod::Audio
    );
    assert_eq!(
        telegram_upload_method(TelegramMediaKind::File, 1),
        TelegramUploadMethod::Document
    );
}

#[test]
fn oversized_managed_artifact_points_to_ui_without_exposing_path() {
    let text = telegram_media_failure_text(
        &test_catalog(),
        Path::new("/private/runtime/report.pdf"),
        LocalMediaPreflightFailure::TooLarge,
        Some(3 * MIB),
        2 * MIB,
        true,
    );
    assert_eq!(text, "report.pdf: 3.00 over 2; open UI");
    assert!(!text.contains("/private/runtime"));
}

#[test]
fn unmanaged_file_failure_requests_retry_instead_of_claiming_ui_copy() {
    let text =
        telegram_media_provider_failure_text(&test_catalog(), Path::new("/tmp/report.pdf"), false);
    assert_eq!(text, "report.pdf: provider failed; retry");
}

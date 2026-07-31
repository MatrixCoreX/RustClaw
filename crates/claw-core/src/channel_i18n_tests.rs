use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::channel_notice::{ChannelNotice, ChannelNoticeSeverity};

fn temp_i18n_path(name: &str, locale: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("agent_channel_i18n_{name}_{locale}_{unique}.toml"))
}

#[test]
fn text_from_path_flattens_dotted_dict_keys() {
    let path = temp_i18n_path("dotted", "en-US");
    std::fs::write(
        &path,
        "[dict]\ncrypto.err.account_access_failed = \"ACCOUNT_ACCESS\"\n\"flat.key\" = \"FLAT\"\n",
    )
    .expect("write i18n");
    let path_text = path.to_string_lossy();

    assert_eq!(
        text_from_path(
            path_text.as_ref(),
            "crypto.err.account_access_failed",
            "fallback"
        ),
        "ACCOUNT_ACCESS"
    );
    assert_eq!(
        text_from_path(path_text.as_ref(), "flat.key", "fallback"),
        "FLAT"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn missing_keys_never_expose_machine_placeholders() {
    let zh_path = temp_i18n_path("missing", "zh-CN");
    let en_path = temp_i18n_path("missing", "en-US");

    let zh = text_from_path(
        zh_path.to_string_lossy().as_ref(),
        "channel.error.missing",
        "message_key=channel.error.missing diagnostic_id={diagnostic_id}",
    );
    let en = text_from_path(
        en_path.to_string_lossy().as_ref(),
        "channel.error.missing",
        "channel.error.missing",
    );

    assert_eq!(zh, safe_generic_text_for_locale("zh-CN"));
    assert_eq!(en, safe_generic_text_for_locale("en-US"));
    assert!(!zh.contains("message_key"));
    assert!(!en.contains("channel.error.missing"));
}

#[test]
fn channel_notice_localization_uses_exact_key_then_safe_locale_fallback() {
    let path = temp_i18n_path("notice", "zh-CN");
    std::fs::write(
        &path,
        "[dict]\n\"channel.notice.working\" = \"处理中：{task_id}\"\n",
    )
    .expect("write i18n");
    let mut notice = ChannelNotice::status(
        "channel.working",
        "channel.notice.working",
        ChannelNoticeSeverity::Info,
    );
    notice
        .params
        .insert("task_id".to_string(), "task-1".to_string());

    let localized =
        localize_channel_notice_from_path(path.to_string_lossy().as_ref(), "zh-CN", &notice);
    assert_eq!(localized.text, "处理中：task-1");
    assert_eq!(
        localized.source,
        ChannelNoticeLocalizationSource::RequestedMessageKey
    );

    notice.message_key = "channel.notice.unknown".to_string();
    let fallback =
        localize_channel_notice_from_path(path.to_string_lossy().as_ref(), "zh-CN", &notice);
    assert_eq!(fallback.text, safe_generic_text_for_locale("zh-CN"));
    assert_eq!(
        fallback.resolved_message_key,
        CHANNEL_NOTICE_SAFE_GENERIC_MESSAGE_KEY
    );
    assert_eq!(
        fallback.source,
        ChannelNoticeLocalizationSource::SafeGenericFallback
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn bundled_common_catalogs_cover_supported_channel_locale_families() {
    for locale in ["en-US", "zh-CN", "ja-JP", "ko-KR"] {
        let text = safe_generic_text_for_locale(locale);
        assert!(
            !text.trim().is_empty(),
            "missing safe fallback for {locale}"
        );
        assert!(!text.contains("message_key="));
        assert_ne!(text, CHANNEL_NOTICE_SAFE_GENERIC_MESSAGE_KEY);
    }
}

#[test]
fn bundled_common_catalogs_cover_every_channel_provider_error_key() {
    let keys = [
        "channel.error.delivery_failed",
        "channel.error.provider_authentication",
        "channel.error.provider_permission_denied",
        "channel.error.provider_rate_limited",
        "channel.error.provider_payload_rejected",
        "channel.error.provider_unavailable",
        "channel.error.provider_invalid_response",
    ];
    for locale in ["en-US", "zh-CN", "ja", "ko"] {
        let dict = common_i18n_dicts()
            .get(locale)
            .unwrap_or_else(|| panic!("missing bundled locale {locale}"));
        for key in keys {
            let value = dict
                .get(key)
                .unwrap_or_else(|| panic!("missing {key} for {locale}"));
            assert!(!value.trim().is_empty(), "empty {key} for {locale}");
            assert_ne!(value, key, "machine key exposed for {locale}");
        }
    }
}

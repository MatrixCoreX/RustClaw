use std::sync::LazyLock;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct QuotaNotice {
    schema_version: u16,
    locale: String,
    quota_exhausted: String,
}

static NOTICE: LazyLock<QuotaNotice> = LazyLock::new(|| {
    let notice: QuotaNotice = toml::from_str(include_str!(
        "../../../configs/provider_failure_notices.toml"
    ))
    .expect("provider_failure_notice_resource_invalid");
    assert_eq!(
        notice.schema_version, 1,
        "provider_failure_notice_schema_invalid"
    );
    assert_eq!(
        notice.locale, "en-US",
        "provider_failure_notice_locale_invalid"
    );
    assert!(
        !notice.quota_exhausted.is_empty()
            && notice.quota_exhausted.trim() == notice.quota_exhausted
            && !notice.quota_exhausted.chars().any(char::is_control),
        "provider_failure_notice_text_invalid"
    );
    notice
});

pub(super) fn quota_exhausted() -> &'static str {
    &NOTICE.quota_exhausted
}

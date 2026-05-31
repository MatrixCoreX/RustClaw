use super::{
    normalize_language_tag, normalized_preference_key, normalized_preference_source_ref_key,
    normalized_preference_value,
};

#[test]
fn memory_intent_language_tag_normalization_is_structural() {
    assert_eq!(normalize_language_tag("zh_CN"), Some("zh-CN".to_string()));
    assert_eq!(normalize_language_tag("ko-KR"), Some("ko-KR".to_string()));
    assert_eq!(normalize_language_tag("fr"), Some("fr-FR".to_string()));
    assert_eq!(normalize_language_tag("EN_us"), Some("en-US".to_string()));
    assert_eq!(normalize_language_tag("中文"), None);
    assert_eq!(normalize_language_tag("-en"), None);
}

#[test]
fn memory_intent_preference_key_allowlist_is_schema_token_based() {
    assert_eq!(
        normalized_preference_key("response_language"),
        Some("response_language".to_string())
    );
    assert_eq!(normalized_preference_key("中文回复"), None);
}

#[test]
fn memory_intent_preference_source_ref_key_is_structural() {
    assert_eq!(
        normalized_preference_source_ref_key("preference:response_language"),
        Some("response_language".to_string())
    );
    assert_eq!(
        normalized_preference_source_ref_key("response_format"),
        Some("response_format".to_string())
    );
    assert_eq!(
        normalized_preference_source_ref_key("language preference"),
        None
    );
}

#[test]
fn memory_intent_preference_values_reject_unstructured_text() {
    assert_eq!(
        normalized_preference_value("response_format", "plain_text"),
        Some("plain_text".to_string())
    );
    assert_eq!(
        normalized_preference_value("response_format", "plain words"),
        None
    );
}

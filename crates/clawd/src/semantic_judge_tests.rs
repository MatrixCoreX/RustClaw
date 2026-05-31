use super::{
    is_publishable_raw_local, looks_like_concrete_delivery_artifact, normalize_classifier_text,
};
use serde_json::Value;

#[test]
fn delivery_text_classifier_schema_drift() {
    const SCHEMA_RAW: &str =
        include_str!("../../../prompts/schemas/delivery_text_classifier.schema.json");
    let schema: Value = serde_json::from_str(SCHEMA_RAW)
        .expect("delivery_text_classifier.schema.json must be valid JSON");
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema.properties must be an object");
    for field in [
        "is_meta_instruction",
        "meta_reason",
        "meta_confidence",
        "publishable",
        "publishable_reason",
        "publishable_confidence",
    ] {
        assert!(
            properties.contains_key(field),
            "schema missing parser field `{field}` under properties — sync prompts/schemas/delivery_text_classifier.schema.json with DeliveryTextClassifierOut",
        );
    }

    let probe = serde_json::json!({
        "is_meta_instruction": false,
        "meta_reason": "user_facing_result",
        "meta_confidence": 0.8,
        "publishable": true,
        "publishable_reason": "meaningful_result",
        "publishable_confidence": 0.9
    });
    let validated = crate::prompt_utils::validate_against_schema::<Value>(
        &probe.to_string(),
        crate::prompt_utils::PromptSchemaId::DeliveryTextClassifier,
    )
    .expect("classifier probe should validate");
    assert_eq!(
        validated.value.get("publishable").and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn normalize_collapses_whitespace_and_trims_trailing_punct() {
    let a = normalize_classifier_text("  Hello\nworld.  ");
    let b = normalize_classifier_text("Hello  world.\n");
    let c = normalize_classifier_text("Hello\tworld");
    assert_eq!(a, "Hello world");
    assert_eq!(b, "Hello world");
    assert_eq!(c, "Hello world");
}

#[test]
fn normalize_handles_cjk_punctuation() {
    assert_eq!(normalize_classifier_text("好的。"), "好的");
    assert_eq!(normalize_classifier_text("好的，"), "好的");
    assert_eq!(normalize_classifier_text("好的！？"), "好的");
}

#[test]
fn normalize_preserves_internal_punctuation() {
    assert_eq!(normalize_classifier_text("This is, fine."), "This is, fine");
}

#[test]
fn local_publishable_rejects_empty_and_filler() {
    assert!(!is_publishable_raw_local(""));
    assert!(!is_publishable_raw_local("  "));
    assert!(!is_publishable_raw_local("a"));
    // 纯标点
    assert!(!is_publishable_raw_local(".....!?"));
    assert!(!is_publishable_raw_local("123 456"));
}

#[test]
fn local_publishable_accepts_real_content() {
    assert!(is_publishable_raw_local(
        "已完成任务，结果保存在 /tmp/out.md"
    ));
    assert!(is_publishable_raw_local(
        "The result is 42 with confidence 0.97."
    ));
    assert!(is_publishable_raw_local("没找到该文件"));
}

#[test]
fn local_delivery_artifact_guard_accepts_paths_and_file_tokens() {
    assert!(looks_like_concrete_delivery_artifact(
        "/home/guagua/rustclaw/document/pwd_line.txt"
    ));
    assert!(looks_like_concrete_delivery_artifact(
        "FILE:/home/guagua/rustclaw/document/pwd_line.txt"
    ));
    assert!(looks_like_concrete_delivery_artifact(
        "C:\\Users\\demo\\pwd_line.txt"
    ));
    assert!(!looks_like_concrete_delivery_artifact("pwd_line.txt"));
    assert!(!looks_like_concrete_delivery_artifact(
        "read pwd_line.txt and summarize it"
    ));
}

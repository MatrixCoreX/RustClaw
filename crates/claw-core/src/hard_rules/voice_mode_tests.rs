use super::*;

#[test]
fn plain_text_without_json_is_rejected() {
    assert_eq!(parse_voice_mode_intent_label("请切到语音回复"), None);
    assert_eq!(parse_voice_mode_intent_label("just text please"), None);
}

#[test]
fn prefers_strict_json_when_confident() {
    let out = parse_voice_mode_intent_decision(
        r#"{"mode":"text","confidence":0.96,"reason":"explicit switch"}"#,
    )
    .expect("decision");
    assert_eq!(out.mode, "text");
    assert_eq!(out.parser_path, "strict_json");
}

#[test]
fn low_confidence_json_returns_none() {
    let out = parse_voice_mode_intent_decision(
        r#"{"mode":"voice","confidence":0.20,"reason":"uncertain"}"#,
    );
    assert_eq!(out, None);
}

#[test]
fn invalid_json_mode_returns_none() {
    let out = parse_voice_mode_intent_decision(r#"{"mode":"chat","confidence":0.99} 切回文字回复"#);
    assert_eq!(out, None);
}

#[test]
fn malformed_output_without_signal_returns_none() {
    let out = parse_voice_mode_intent_decision("n/a ???");
    assert_eq!(out, None);
}

#[test]
fn extracted_json_wrapper_is_accepted() {
    let out = parse_voice_mode_intent_decision(
        r#"答案如下 {"mode":"show","confidence":0.91,"reason":"asks current mode"}"#,
    )
    .expect("decision");
    assert_eq!(out.mode, "show");
    assert_eq!(out.parser_path, "extracted_json");
}

#[test]
fn missing_reason_is_rejected_by_schema() {
    let out = parse_voice_mode_intent_decision(r#"{"mode":"text","confidence":0.98}"#);
    assert_eq!(out, None);
}

#[test]
fn additional_property_is_rejected_by_schema() {
    let out = parse_voice_mode_intent_decision(
        r#"{"mode":"text","confidence":0.98,"reason":"explicit switch","extra":true}"#,
    );
    assert_eq!(out, None);
}

#[test]
fn voice_mode_intent_schema_drift() {
    let schema: JsonValue =
        serde_json::from_str(VOICE_MODE_INTENT_SCHEMA_RAW).expect("schema json");
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("properties");
    for field in ["mode", "confidence", "reason"] {
        assert!(properties.contains_key(field), "missing property {field}");
        assert!(
            schema_requires_field(&schema, field),
            "missing required {field}"
        );
    }

    let mode_enum = properties
        .get("mode")
        .and_then(|v| v.get("enum"))
        .and_then(|v| v.as_array())
        .expect("mode enum");
    for token in mode_enum {
        let token = token.as_str().expect("enum token");
        assert_eq!(
            parse_mode_token(token),
            Some(token),
            "schema enum token not recognized by parser: {token}"
        );
    }
}

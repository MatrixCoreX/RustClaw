use super::*;

#[test]
fn resume_error_decoder_preserves_model_presentation_and_failure_context() {
    let presentation = r#"{"status":"error","total":null,"error_code":"context_source_compacted"}"#;
    let context = serde_json::json!({"user_error":presentation,"remaining_actions":[],"failed_step":{"index":1}});
    let raw = format!("{}{}", crate::RESUME_CONTEXT_ERROR_PREFIX, context);
    let (reply, decoded) = parse_resume_context_error(&raw).unwrap();
    assert_eq!(reply, presentation);
    assert_eq!(decoded, context);
}

#[test]
fn resume_error_missing_presentation_uses_public_machine_state_not_raw_context() {
    for user_error in [
        serde_json::Value::Null,
        serde_json::json!(" "),
        serde_json::json!(3),
    ] {
        let context = serde_json::json!({"user_error":user_error,"failed_step":{"error":"private-internal-body"}});
        let raw = format!("{}{}", crate::RESUME_CONTEXT_ERROR_PREFIX, context);
        let (reply, decoded) = parse_resume_context_error(&raw).unwrap();
        let value: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(value["status"], "error");
        assert_eq!(value["presentation_status"], "unavailable");
        assert!(!reply.contains("private-internal-body"));
        assert_eq!(decoded, context);
    }
    assert!(parse_resume_context_error("ordinary failure").is_none());
    assert!(parse_resume_context_error(crate::RESUME_CONTEXT_ERROR_PREFIX).is_none());
}

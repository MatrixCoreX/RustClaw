use super::*;

#[test]
fn public_failure_json_preserves_requested_result_fields_without_internal_wrappers() {
    assert!(public_failure_object_allowed(
        r#"{"total":null,"indices":null,"status":"error","error_code":"context_source_compacted","retryable":false}"#
    ));
    assert!(public_failure_object_allowed(
        r#"{"result":{"status":"failed","value":null},"details":[1,true]}"#
    ));
    for text in [
        "[]",
        "{bad}",
        r#"{"headers":{"authorization":"hidden"}}"#,
        r#"{"resume_context":{}}"#,
        r#"{"nested":{"raw_response":"provider body"}}"#,
        r#"{"nested":{" raw-response ":"provider body"}}"#,
        r#"{"execution.binding":{}}"#,
        r#"{"details":"prompt_source=private"}"#,
        r#"{"password":"hidden"}"#,
        r#"{"nested":{"client_secret":"hidden"}}"#,
        r#"{"status":"ok"}"#,
        r#"{"error":"{\"raw_response\":\"provider body\"}"}"#,
    ] {
        assert!(!public_failure_object_allowed(text), "{text}");
    }
    assert!(!public_failure_object_allowed(
        &serde_json::json!({"details":"x".repeat(25 * 1024)}).to_string()
    ));
}

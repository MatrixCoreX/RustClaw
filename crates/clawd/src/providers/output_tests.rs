use super::*;

#[test]
fn raw_response_sanitizer_preserves_visible_fields_without_hidden_reasoning() {
    let raw = json!({
        "choices": [{
            "message": {
                "content": "<think>private reasoning</think>{\"pass\":true}",
                "reasoning_content": "private field"
            }
        }],
        "usage": {"total_tokens": 10}
    })
    .to_string();

    let (safe, changed) = sanitize_provider_raw_response(&raw);
    let value: Value = serde_json::from_str(&safe).expect("safe JSON");

    assert!(changed);
    assert_eq!(
        value.pointer("/choices/0/message/content"),
        Some(&Value::String("{\"pass\":true}".to_string()))
    );
    assert!(value
        .pointer("/choices/0/message/reasoning_content")
        .is_none());
    assert_eq!(value.pointer("/usage/total_tokens"), Some(&json!(10)));
}

#[test]
fn raw_response_sanitizer_handles_json_lines() {
    let raw = [
        json!({"choices": [{"delta": {"content": "<think>hidden</think>visible"}}]}).to_string(),
        json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}).to_string(),
    ]
    .join("\n");

    let (safe, changed) = sanitize_provider_raw_response(&raw);

    assert!(changed);
    assert!(!safe.contains("hidden"));
    assert!(safe.contains("visible"));
    assert!(safe.contains("finish_reason"));
}

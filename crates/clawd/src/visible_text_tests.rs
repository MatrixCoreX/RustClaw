use super::sanitize_user_visible_text;

#[test]
fn sanitizes_ansi_and_sensitive_url_params() {
    let raw = "\u{1b}[32mconnected\u{1b}[0m to wss://host/ws?device_id=123&access_key=abc123&service_id=7&ticket=deadbeef";

    let sanitized = sanitize_user_visible_text(raw);

    assert_eq!(
        sanitized,
        "connected to wss://host/ws?device_id=123&access_key=[REDACTED]&service_id=7&ticket=[REDACTED]"
    );
}

#[test]
fn sanitizes_json_escaped_ansi_and_sensitive_fields() {
    let raw = r#"{"excerpt":"1|\u001b[32mok\u001b[0m token=abc123456789","api_secret":"plain"}"#;

    let sanitized = sanitize_user_visible_text(raw);

    assert!(!sanitized.contains("\\u001b"));
    assert!(sanitized.contains("token=[REDACTED]"));
    assert!(sanitized.contains(r#""api_secret":"[REDACTED]""#));
    assert!(!sanitized.contains("abc123456789"));
    assert!(!sanitized.contains("plain"));
}

#[test]
fn keeps_i18n_message_key_machine_fields() {
    let raw = "message_key=crypto.err.account_access_failed api_key=secret-token";

    let sanitized = sanitize_user_visible_text(raw);

    assert!(sanitized.contains("message_key=crypto.err.account_access_failed"));
    assert!(sanitized.contains("api_key=[REDACTED]"));
    assert!(!sanitized.contains("secret-token"));
}

#[test]
fn sanitizes_structured_skill_error_payloads() {
    let raw = r#"已尝试访问文件，但执行失败：__RC_SKILL_ERROR__:{"skill":"archive_basic","error_kind":"unknown","error_text":"archive is required","text":null}。"#;

    let sanitized = sanitize_user_visible_text(raw);

    assert_eq!(
        sanitized,
        "已尝试访问文件，但执行失败：archive is required。"
    );
    assert!(!sanitized.contains("__RC_SKILL_ERROR__"));
    assert!(!sanitized.contains("\"skill\""));
}

#[test]
fn malformed_structured_skill_error_payload_does_not_leak_marker_tail() {
    let raw = r#"执行失败：__RC_SKILL_ERROR__:{"skill":"archive_basic","error_text":"broken""#;

    let sanitized = sanitize_user_visible_text(raw);

    assert_eq!(sanitized, "执行失败：skill execution failed");
    assert!(!sanitized.contains("__RC_SKILL_ERROR__"));
    assert!(!sanitized.contains("archive_basic"));
}

#[test]
fn compacts_internal_model_io_json_lines() {
    let raw = r#"{"task_id":"task-1","vendor":"minimax","model":"MiniMax-M2.7","status":"ok","prompt":"SECRET_PROMPT_SHOULD_NOT_SHOW","raw_response":"RAW_RESPONSE_SHOULD_NOT_SHOW","request_payload":{"messages":[{"role":"user","content":"PAYLOAD_SHOULD_NOT_SHOW"}]},"response":"{\"steps\":[]}","usage":{"total_tokens":12}}"#;

    let sanitized = sanitize_user_visible_text(raw);

    assert!(sanitized.contains("task-1"));
    assert!(sanitized.contains("omitted_fields"));
    assert!(sanitized.contains("response_preview"));
    assert!(!sanitized.contains("SECRET_PROMPT_SHOULD_NOT_SHOW"));
    assert!(!sanitized.contains("RAW_RESPONSE_SHOULD_NOT_SHOW"));
    assert!(!sanitized.contains("PAYLOAD_SHOULD_NOT_SHOW"));
}

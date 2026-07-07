use super::*;

fn test_runtime_config() -> XRuntimeConfig {
    XRuntimeConfig {
        use_xurl: true,
        xurl_bin: "xurl".to_string(),
        xurl_app: None,
        xurl_auth: None,
        xurl_username: None,
        xurl_timeout_seconds: 30,
        require_explicit_send: true,
        max_text_chars: 280,
    }
}

#[test]
fn dry_run_extra_uses_machine_fields_without_live_publish() {
    let input = XActionInput {
        text: "Daily market note".to_string(),
        dry_run: true,
        send: false,
    };

    let extra = x_post_extra(&input, &test_runtime_config(), "dry_run", false);

    assert_eq!(extra.get("status").and_then(Value::as_str), Some("ok"));
    assert_eq!(extra.get("action").and_then(Value::as_str), Some("post"));
    assert_eq!(extra.get("source_skill").and_then(Value::as_str), Some("x"));
    assert_eq!(
        extra.get("outcome").and_then(Value::as_str),
        Some("dry_run")
    );
    assert_eq!(extra.get("dry_run").and_then(Value::as_bool), Some(true));
    assert_eq!(extra.get("send").and_then(Value::as_bool), Some(false));
    assert_eq!(extra.get("published").and_then(Value::as_bool), Some(false));
    assert_eq!(
        extra.get("text_char_count").and_then(Value::as_u64),
        Some(17)
    );
}

#[test]
fn invalid_flag_combo_returns_structured_error_kind() {
    let err = parse_input(json!({
        "text": "Daily market note",
        "send": true,
        "dry_run": true
    }))
    .unwrap_err();

    assert_eq!(err.kind, "invalid_input");
    assert_eq!(
        err.extra().get("error_kind").and_then(Value::as_str),
        Some("invalid_input")
    );
}

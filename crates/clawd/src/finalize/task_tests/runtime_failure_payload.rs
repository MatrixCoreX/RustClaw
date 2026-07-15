use super::*;

#[test]
fn ask_runtime_failure_payload_is_machine_readable() {
    let payload: serde_json::Value = serde_json::from_str(&ask_runtime_failure_machine_payload(
        r#"provider=vendor-minimax failed: http 429: {"error":{"type":"rate_limit_error"}}"#,
    ))
    .unwrap();

    assert_eq!(payload["message_key"], "clawd.msg.ask_runtime_failure");
    assert_eq!(payload["reason_code"], "provider_rate_limited");
    assert_eq!(payload["status_code"], "provider_rate_limited");
    assert_eq!(payload["failure_attribution"], "provider_gap");
    assert_eq!(payload["retryable"], false);
    assert_eq!(payload["raw_error_present"], true);
    assert_eq!(payload["provider_error_class"], "rate_limited");
    assert_eq!(payload["external_provider_blocked"], true);
    assert!(payload.pointer("/error_summary").is_none());
}

#[test]
fn ask_runtime_failure_observed_facts_use_machine_payload_fields() {
    let facts = machine_payload_observed_facts(&ask_runtime_failure_machine_payload(
        r#"provider=vendor-minimax failed: http 429: {"error":{"type":"rate_limit_error"}}"#,
    ));

    assert!(facts.contains(&"message_key: clawd.msg.ask_runtime_failure".to_string()));
    assert!(facts.contains(&"status_code: provider_rate_limited".to_string()));
    assert!(facts.contains(&"failure_attribution: provider_gap".to_string()));
    assert!(facts.contains(&"retryable: false".to_string()));
    assert!(facts.contains(&"raw_error_present: true".to_string()));
    assert!(facts.contains(&"provider_error_class: rate_limited".to_string()));
    assert!(facts.contains(&"external_provider_blocked: true".to_string()));
    assert!(!facts.iter().any(|fact| fact.starts_with("error_summary:")));
}

#[test]
fn ask_runtime_failure_default_text_preserves_provider_blocker_payload() {
    let text = ask_runtime_failure_default_text(
        r#"provider=vendor-qwen failed: http 404: {"error":{"code":"model_not_found","type":"invalid_request_error"}}"#,
    );
    let payload: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(payload["message_key"], "clawd.msg.ask_runtime_failure");
    assert_eq!(payload["reason_code"], "provider_model_unavailable");
    assert_eq!(payload["status_code"], "provider_model_unavailable");
    assert_eq!(payload["provider_error_class"], "model_unavailable");
    assert_eq!(payload["external_provider_blocked"], true);
}

#[test]
fn ask_runtime_failure_payload_classifies_provider_quota_blocker() {
    let payload: serde_json::Value = serde_json::from_str(&ask_runtime_failure_machine_payload(
        r#"provider=vendor-mimo failed: http 429: {"error":{"code":"429","type":"limitation"}}"#,
    ))
    .unwrap();

    assert_eq!(payload["reason_code"], "provider_quota_exceeded");
    assert_eq!(payload["status_code"], "provider_quota_exceeded");
    assert_eq!(payload["provider_error_class"], "quota_exceeded");
    assert_eq!(payload["external_provider_blocked"], true);
    assert_eq!(payload["provider_http_status"], 429);
    assert_eq!(payload["provider_error_code"], "429");
    assert_eq!(payload["provider_error_type"], "limitation");
}

#[test]
fn ask_runtime_failure_payload_classifies_provider_account_blocker() {
    let payload: serde_json::Value = serde_json::from_str(&ask_runtime_failure_machine_payload(
        r#"provider=vendor-qwen failed: http 400: {"error":{"code":"Arrearage","type":"Arrearage"}}"#,
    ))
    .unwrap();

    assert_eq!(payload["reason_code"], "provider_account_blocked");
    assert_eq!(payload["status_code"], "provider_account_blocked");
    assert_eq!(payload["provider_error_class"], "account_blocked");
    assert_eq!(payload["external_provider_blocked"], true);
    assert_eq!(payload["provider_error_code"], "Arrearage");
}

#[test]
fn ask_runtime_failure_payload_classifies_provider_model_unavailable() {
    let payload: serde_json::Value = serde_json::from_str(&ask_runtime_failure_machine_payload(
        r#"provider=vendor-qwen failed: http 404: {"error":{"code":"model_not_found","type":"invalid_request_error"}}"#,
    ))
    .unwrap();

    assert_eq!(payload["reason_code"], "provider_model_unavailable");
    assert_eq!(payload["status_code"], "provider_model_unavailable");
    assert_eq!(payload["provider_error_class"], "model_unavailable");
    assert_eq!(payload["external_provider_blocked"], true);
    assert_eq!(payload["provider_http_status"], 404);
}

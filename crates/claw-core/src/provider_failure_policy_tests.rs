use super::*;

#[test]
fn quota_exhaustion_terminates_without_background_wait_or_retry() {
    let policy = ProviderFailureClass::QuotaExhausted.policy();

    assert_eq!(policy.failure_class.as_str(), "quota_exhausted");
    assert!(!policy.provider_retryable);
    assert!(!policy.provider_blocker);
    assert_eq!(policy.retry_policy, "none");
    assert_eq!(policy.retry_after_seconds, None);
    assert_eq!(policy.waiting_state, "terminal");
    assert!(!policy.checkpoint_required);
    assert_eq!(policy.resume_reason, None);
    assert_eq!(policy.resume_entrypoint, None);
}

#[test]
fn every_failure_class_round_trips_through_its_machine_token() {
    for failure_class in ProviderFailureClass::ALL {
        assert_eq!(
            ProviderFailureClass::from_str(failure_class.as_str()),
            Some(failure_class)
        );
    }
    assert_eq!(ProviderFailureClass::from_str("quota exhausted"), None);
}

#[test]
fn only_quota_exhaustion_has_a_fixed_terminal_notice() {
    for class in ProviderFailureClass::ALL {
        assert_eq!(
            class.terminal_notice_text().is_some(),
            class == ProviderFailureClass::QuotaExhausted
        );
    }
    let text = ProviderFailureClass::QuotaExhausted
        .terminal_notice_text()
        .unwrap();
    assert_eq!(text, "API quota exhausted. Task terminated.");
    assert!(text.is_ascii());
}

#[test]
fn terminal_quota_authority_requires_the_complete_machine_lifecycle() {
    let lifecycle = serde_json::json!({
        "schema_version":1,"state":"failed","reason_code":PROVIDER_QUOTA_TERMINATED,
        "retryable":false,"provider_status":{"status_code":"quota_exhausted","external_provider_blocked":true}
    });
    assert!(is_terminal_quota_lifecycle(&lifecycle));
    for (pointer, value) in [
        ("/schema_version", serde_json::json!(2)),
        ("/state", serde_json::json!("waiting")),
        ("/reason_code", serde_json::json!("provider_unavailable")),
        ("/retryable", serde_json::json!(true)),
        (
            "/provider_status/status_code",
            serde_json::json!("rate_limited"),
        ),
        (
            "/provider_status/external_provider_blocked",
            serde_json::json!(false),
        ),
    ] {
        let mut invalid = lifecycle.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(!is_terminal_quota_lifecycle(&invalid), "{pointer}");
        *invalid.pointer_mut(pointer).unwrap() = serde_json::Value::Null;
        assert!(!is_terminal_quota_lifecycle(&invalid), "missing {pointer}");
    }
    assert!(!is_terminal_quota_lifecycle(
        &serde_json::json!({"text":"quota_exhausted"})
    ));
}

#[test]
fn temporary_provider_failures_keep_their_recoverable_wait_policies() {
    assert_eq!(
        ProviderFailureClass::RateLimited.background_wait_seconds(),
        Some(60)
    );
    for class in [
        ProviderFailureClass::Timeout,
        ProviderFailureClass::TransportRetryable,
        ProviderFailureClass::ProviderRetryableResponse,
    ] {
        assert_eq!(class.background_wait_seconds(), Some(30));
        assert!(class.policy().checkpoint_required);
    }
}

#[test]
fn terminal_failures_do_not_publish_wait_or_checkpoint_contracts() {
    for failure_class in [
        ProviderFailureClass::ProviderNonRetryableBusiness,
        ProviderFailureClass::LocalNonRetryable,
    ] {
        let policy = failure_class.policy();
        assert!(!policy.provider_blocker);
        assert_eq!(policy.retry_policy, "none");
        assert_eq!(policy.retry_after_seconds, None);
        assert_eq!(policy.waiting_state, "terminal");
        assert!(!policy.checkpoint_required);
        assert_eq!(policy.resume_reason, None);
        assert_eq!(policy.resume_entrypoint, None);
    }
}

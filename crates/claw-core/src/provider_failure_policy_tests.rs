use super::*;

#[test]
fn quota_exhaustion_uses_background_wait_checkpoint_policy() {
    let policy = ProviderFailureClass::QuotaExhausted.policy();

    assert_eq!(policy.failure_class.as_str(), "quota_exhausted");
    assert!(!policy.provider_retryable);
    assert!(policy.provider_blocker);
    assert_eq!(policy.retry_policy, "background_wait");
    assert_eq!(policy.retry_after_seconds, Some(10_800));
    assert_eq!(policy.waiting_state, "waiting");
    assert!(policy.checkpoint_required);
    assert_eq!(
        policy.resume_reason,
        Some("provider_blocker_wait_background")
    );
    assert_eq!(policy.resume_entrypoint, Some("next_planner_round"));
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

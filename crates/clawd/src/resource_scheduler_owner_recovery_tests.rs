use super::*;

#[test]
fn recovery_startup_barrier_refuses_foreground_and_background_admission() {
    let broker = ResourceBroker::for_owner_recovery();
    let alias = broker.clone();
    for background in [false, true] {
        let error = if background {
            alias.try_acquire_background_for_estimate_key(None, 1, None)
        } else {
            alias.try_acquire(None, 1)
        }
        .err()
        .unwrap();
        assert!(!error.admitted);
        assert_eq!(error.wait_reason, Some("owner_recovery_pending"));
    }
    assert_eq!(broker.status().active_leases, 0);
    assert!(broker.state.lock().unwrap().owner_recovery_pending);
}

#[test]
fn ordinary_broker_has_no_implicit_recovery_or_authority_grant() {
    let broker = ResourceBroker::default();
    assert!(!broker.state.lock().unwrap().owner_recovery_pending);
    assert!(broker.state.lock().unwrap().owner_custody.is_empty());
}

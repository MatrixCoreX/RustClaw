use super::*;
#[test]
fn status_polling_keeps_expiration_pending_for_the_lifecycle_task() {
    let mut client = VaultClient {
        worker: None,
        input: None,
        output: None,
        #[cfg(windows)]
        guard: None,
        next: 0,
        failure: None,
        was_unlocked: true,
        revoked: false,
    };
    client.observe_lock(true);
    assert!(!client.revoked);
    client.observe_lock(false);
    client.observe_lock(false);
    assert!(std::mem::take(&mut client.revoked));
    client.observe_lock(false);
    assert!(!client.revoked);
}

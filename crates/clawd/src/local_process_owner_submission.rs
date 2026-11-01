use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

const OPEN: u8 = 0;
const SUBMITTING: u8 = 1;
const ABANDONED: u8 = 2;

// Keep the guard in the async caller, not in its blocking operation. Neither
// side is a completion proof, permission grant or instruction to retry.
#[must_use]
pub(crate) struct HostSubmissionGuard(Arc<AtomicU8>);

pub(super) struct HostSubmissionPermit(Arc<AtomicU8>);

impl HostSubmissionGuard {
    pub(super) fn pair() -> (Self, HostSubmissionPermit) {
        let state = Arc::new(AtomicU8::new(OPEN));
        (Self(Arc::clone(&state)), HostSubmissionPermit(state))
    }
}

impl Drop for HostSubmissionGuard {
    fn drop(&mut self) {
        // After submission wins, caller loss cannot revoke a possibly running
        // operation or release its reservations and immutable package version.
        let _ = self
            .0
            .compare_exchange(OPEN, ABANDONED, Ordering::AcqRel, Ordering::Acquire);
    }
}

impl HostSubmissionPermit {
    pub(super) fn is_open(&self) -> bool {
        self.0.load(Ordering::Acquire) == OPEN
    }

    pub(super) fn enter(self) -> bool {
        self.0
            .compare_exchange(OPEN, SUBMITTING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

#[cfg(test)]
#[path = "local_process_owner_submission_tests.rs"]
mod tests;

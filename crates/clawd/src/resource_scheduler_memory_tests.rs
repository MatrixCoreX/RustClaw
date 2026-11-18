use std::cell::Cell;

use super::sample_durable_process_memory;
use crate::local_process_job::ProcessIdentityState;

fn assert_unverified_sample_is_denied(identity: ProcessIdentityState) {
    let calls = Cell::new(0);
    let result = sample_durable_process_memory(identity, Some(42), |_| {
        calls.set(calls.get() + 1);
        Some(900)
    });
    assert_eq!(calls.get(), 0, "unverified collector must not run");
    assert_eq!(result, None);
}

#[test]
fn missing_process_cannot_contribute_a_memory_sample() {
    assert_unverified_sample_is_denied(ProcessIdentityState::Missing);
}

#[test]
fn mismatched_process_cannot_contribute_a_memory_sample() {
    assert_unverified_sample_is_denied(ProcessIdentityState::IdentityMismatch);
}

#[test]
fn unknown_process_cannot_contribute_a_memory_sample() {
    assert_unverified_sample_is_denied(ProcessIdentityState::Unknown);
}

#[test]
fn absent_pid_never_invokes_the_memory_collector() {
    for identity in [
        ProcessIdentityState::AliveVerified,
        ProcessIdentityState::Missing,
        ProcessIdentityState::IdentityMismatch,
        ProcessIdentityState::Unknown,
    ] {
        assert_eq!(
            sample_durable_process_memory(identity, None, |_| panic!("unexpected collector")),
            None
        );
    }
}

#[test]
fn verified_sample_preserves_pid_value_and_exactly_one_observation() {
    let calls = Cell::new(0);
    let sample =
        sample_durable_process_memory(ProcessIdentityState::AliveVerified, Some(42), |pid| {
            assert_eq!(pid, 42);
            calls.set(calls.get() + 1);
            Some(384)
        });
    assert_eq!(sample, Some(384));
    assert_eq!(calls.get(), 1);
}

#[test]
fn verified_zero_memory_is_not_changed_to_unavailable() {
    assert_eq!(
        sample_durable_process_memory(ProcessIdentityState::AliveVerified, Some(42), |_| {
            Some(0)
        }),
        Some(0)
    );
}

#[test]
fn unavailable_verified_memory_is_not_changed_to_zero() {
    assert_eq!(
        sample_durable_process_memory(ProcessIdentityState::AliveVerified, Some(42), |_| None),
        None
    );
}

#[test]
fn verified_memory_does_not_truncate_a_large_measurement() {
    assert_eq!(
        sample_durable_process_memory(ProcessIdentityState::AliveVerified, Some(42), |_| {
            Some(u64::MAX)
        }),
        Some(u64::MAX)
    );
}

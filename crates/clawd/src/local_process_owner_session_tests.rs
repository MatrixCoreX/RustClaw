use std::cell::Cell;

use super::super::exit::ChildExit;
use super::super::instance::{ProcessBirthIdentity, ProcessStart};
use super::super::owner_protocol::{OwnerCompletion, OwnerPhase};
use super::*;

fn session() -> OwnerSession {
    OwnerSession::new(OwnerCredential {
        job_id: uuid::Uuid::new_v4(),
        nonce: uuid::Uuid::new_v4(),
        owner: ProcessBirthIdentity {
            pid: 42,
            boot_id: uuid::Uuid::new_v4(),
            started: ProcessStart::LinuxTicks { ticks: 7 },
        },
    })
    .unwrap()
}

fn request(session: &OwnerSession, request_id: u64, action: OwnerAction) -> OwnerRequest {
    OwnerRequest {
        schema_version: 1,
        credential: session.credential(),
        request_id,
        action,
    }
}

fn running() -> OwnerSnapshot {
    OwnerSnapshot {
        phase: OwnerPhase::Running,
        completion: None,
        root_exit: None,
    }
}

fn complete() -> OwnerSnapshot {
    OwnerSnapshot {
        phase: OwnerPhase::Terminal,
        completion: Some(OwnerCompletion::Completed),
        root_exit: Some(ChildExit {
            code: Some(7),
            signal: None,
        }),
    }
}

#[test]
fn owner_session_binds_the_job_nonce_and_full_owner_birth_before_execution() {
    let mut session = session();
    let calls = Cell::new(0);
    for field in 0..5 {
        let mut input = request(&session, 1, OwnerAction::Cancel);
        match field {
            0 => input.credential.job_id = uuid::Uuid::new_v4(),
            1 => input.credential.nonce = uuid::Uuid::new_v4(),
            2 => input.credential.owner.pid += 1,
            3 => input.credential.owner.boot_id = uuid::Uuid::new_v4(),
            _ => input.credential.owner.started = ProcessStart::LinuxTicks { ticks: 8 },
        }
        assert_eq!(
            session.handle(&input, |_| {
                calls.set(calls.get() + 1);
                Ok(running())
            }),
            Err(OwnerFailure::CredentialMismatch)
        );
        assert!(session.last_reply.is_none());
    }
    assert_eq!(calls.get(), 0);
}

#[test]
fn owner_session_invalid_contracts_cannot_consume_sequence_or_call_execution() {
    let mut session = session();
    let mut input = request(&session, 1, OwnerAction::Cancel);
    input.schema_version = 2;
    assert_eq!(
        session.handle(&input, |_| panic!("unexpected execution")),
        Err(OwnerFailure::UnsupportedSchema)
    );
    input.schema_version = 1;
    input.credential.nonce = uuid::Uuid::nil();
    assert_eq!(
        session.handle(&input, |_| panic!("unexpected execution")),
        Err(OwnerFailure::InvalidCredential)
    );
    assert!(session.last_reply.is_none());
    let mut invalid = session.credential();
    invalid.nonce = uuid::Uuid::nil();
    assert!(matches!(
        OwnerSession::new(invalid),
        Err(OwnerFailure::InvalidCredential)
    ));
}

#[test]
fn owner_session_exact_replay_returns_the_original_ack_without_observing_again() {
    let mut session = session();
    let calls = Cell::new(0);
    let input = request(&session, 1, OwnerAction::Inspect);
    let first = session
        .handle(&input, |action| {
            assert_eq!(action, OwnerAction::Inspect);
            calls.set(calls.get() + 1);
            Ok(running())
        })
        .unwrap();
    for _ in 0..5 {
        assert_eq!(
            session.handle(&input, |_| {
                calls.set(calls.get() + 1);
                Ok(complete())
            }),
            Ok(first)
        );
    }
    assert_eq!(calls.get(), 1);
    let next = request(&session, 2, OwnerAction::Inspect);
    assert_eq!(
        session
            .handle(&next, |_| {
                calls.set(calls.get() + 1);
                Ok(complete())
            })
            .unwrap()
            .outcome,
        OwnerOutcome::Ok {
            snapshot: complete()
        }
    );
    assert_eq!(calls.get(), 2);
}

#[test]
fn owner_session_rejects_gaps_old_requests_and_conflicting_replays() {
    let mut session = session();
    for number in [0, 2, u64::MAX] {
        let input = request(&session, number, OwnerAction::Inspect);
        assert_eq!(
            session.handle(&input, |_| panic!("unexpected execution")),
            Err(OwnerFailure::SequenceGap)
        );
    }
    let first = request(&session, 1, OwnerAction::Inspect);
    session.handle(&first, |_| Ok(running())).unwrap();
    let conflict = request(&session, 1, OwnerAction::Cancel);
    assert_eq!(
        session.handle(&conflict, |_| panic!("unexpected execution")),
        Err(OwnerFailure::ReplayConflict)
    );
    let second = request(&session, 2, OwnerAction::Inspect);
    session.handle(&second, |_| Ok(running())).unwrap();
    assert_eq!(
        session.handle(&first, |_| panic!("unexpected execution")),
        Err(OwnerFailure::ReplayConflict)
    );
}

#[test]
fn owner_session_caches_failures_without_repeating_the_operation() {
    let mut session = session();
    let input = request(&session, 1, OwnerAction::Cancel);
    let first = session
        .handle(&input, |_| Err(OwnerFailure::OwnershipLost))
        .unwrap();
    assert_eq!(
        first.outcome,
        OwnerOutcome::Error {
            error_code: OwnerFailure::OwnershipLost
        }
    );
    assert_eq!(
        session.handle(&input, |_| panic!("failed operation repeated")),
        Ok(first)
    );
}

#[test]
fn owner_session_sequence_overflow_cannot_reset_or_replay_an_action() {
    let mut session = session();
    session.last_reply = Some((
        OwnerAction::Inspect,
        OwnerReply {
            schema_version: 1,
            request_id: u64::MAX,
            outcome: OwnerOutcome::Ok {
                snapshot: complete(),
            },
        },
    ));
    let exact = request(&session, u64::MAX, OwnerAction::Inspect);
    assert!(session
        .handle(&exact, |_| panic!("unexpected execution"))
        .is_ok());
    let wrapped = request(&session, 0, OwnerAction::Cancel);
    assert_eq!(
        session.handle(&wrapped, |_| panic!("unexpected execution")),
        Err(OwnerFailure::SequenceExhausted)
    );
}

#[test]
fn owner_session_new_invocation_refuses_an_old_credential_for_the_same_job() {
    let first = session();
    let mut next = first.credential();
    next.nonce = uuid::Uuid::new_v4();
    let mut second = OwnerSession::new(next).unwrap();
    let old = request(&first, 1, OwnerAction::Cancel);
    assert_eq!(
        second.handle(&old, |_| panic!("unexpected execution")),
        Err(OwnerFailure::CredentialMismatch)
    );
    assert!(second.last_reply.is_none());
}

#[test]
fn owner_session_invalid_terminal_evidence_is_cached_as_failure_not_reexecuted() {
    let mut session = session();
    let input = request(&session, 1, OwnerAction::Cancel);
    let mut invalid = complete();
    invalid.root_exit = None;
    let failed = session.handle(&input, |_| Ok(invalid)).unwrap();
    assert_eq!(
        failed.outcome,
        OwnerOutcome::Error {
            error_code: OwnerFailure::InvalidState
        }
    );
    assert_eq!(
        session.handle(&input, |_| panic!("invalid result caused action replay")),
        Ok(failed)
    );
}

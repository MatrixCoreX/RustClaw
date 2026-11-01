use super::super::super::owner_store::tests::{binding, credential};
use super::*;
use std::os::unix::net::UnixStream;

fn pair() -> (LaunchChannel, LaunchChannel) {
    let (first, second) = UnixStream::pair().unwrap();
    (
        LaunchChannel::new(first).unwrap(),
        LaunchChannel::new(second).unwrap(),
    )
}

fn pending(channel: LaunchChannel, credential: OwnerCredential) -> OwnerPendingPreparation {
    // Synthetic private fixture for a fresh job; no other start exists. Actual
    // validated-bootstrap admission is covered by the pinned executable tests.
    unsafe {
        OwnerPendingPreparation::new(channel, credential, binding(), Duration::from_millis(1))
    }
    .unwrap()
}

#[test]
fn unstarted_preparation_refuses_a_different_writer_or_empty_binding() {
    let original = credential(uuid::Uuid::new_v4());
    let mut other = original.clone();
    other.owner.boot_id = uuid::Uuid::new_v4();
    for (credential, binding) in [(other, binding()), (original, [0; 32])] {
        let (owner, peer) = pair();
        assert!(unsafe {
            OwnerPendingPreparation::new(owner, credential, binding, Duration::from_millis(1))
        }
        .is_err());
        drop(peer);
    }
}

#[test]
fn missing_package_frame_consumes_preparation_without_a_launch() {
    let original = credential(uuid::Uuid::new_v4());
    let (owner, peer) = pair();
    let owner = pending(owner, original.clone());
    drop(peer);
    let abandoned = owner
        .receive(Instant::now() + Duration::from_secs(1))
        .err()
        .unwrap();
    assert!(matches!(
        abandoned.error,
        OwnerPreparationFailure::Transport(LaunchChannelFailure::TransportUnavailable)
    ));
    assert!(abandoned.package.is_none());
    assert!(abandoned.evidence.observed() == (&original, &binding()));
}

#[test]
fn missing_launch_consumes_preparation_after_the_package_frame() {
    let original = credential(uuid::Uuid::new_v4());
    let (owner, peer) = pair();
    let owner = pending(owner, original.clone());
    drop(
        peer.send_package_lease(None, &original, Instant::now() + Duration::from_secs(1))
            .unwrap(),
    );
    let abandoned = owner
        .receive(Instant::now() + Duration::from_secs(1))
        .err()
        .unwrap();
    assert!(matches!(
        abandoned.error,
        OwnerPreparationFailure::Transport(LaunchChannelFailure::TransportUnavailable)
    ));
    assert!(abandoned.evidence.observed() == (&original, &binding()));
}

#[test]
fn successful_receive_yields_only_a_prepared_launch_before_commit() {
    use super::super::super::process_launch::{PayloadIo, ProcessLaunch};
    use crate::process_invocation::ProcessInvocation;
    let original = credential(uuid::Uuid::new_v4());
    let (owner, peer) = pair();
    let owner = pending(owner, original.clone());
    let launch = ProcessLaunch::new(
        ProcessInvocation::new("/bin/true"),
        std::path::PathBuf::from("/"),
        vec![],
        PayloadIo {
            stdin: None,
            stdout: None,
            stderr: None,
        },
    )
    .unwrap();
    let peer = peer
        .send_package_lease(None, &original, Instant::now() + Duration::from_secs(1))
        .unwrap()
        .send(
            launch,
            original.clone(),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
    let (prepared, package) = match owner.receive(Instant::now() + Duration::from_secs(1)) {
        Ok(value) => value,
        Err(_) => panic!("valid_preparation_rejected"),
    };
    assert!(package.is_none());
    let evidence = prepared.abandon();
    assert!(evidence.observed() == (&original, &binding()));
    drop(peer);
}

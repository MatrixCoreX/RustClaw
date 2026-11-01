use super::*;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::time::Duration;

const BINDING: [u8; 32] = [83; 32];

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn credential() -> OwnerCredential {
    OwnerRuntime::prepare_credential(uuid::Uuid::new_v4(), uuid::Uuid::new_v4()).unwrap()
}
fn pair() -> (LaunchChannel, LaunchChannel) {
    let (first, second) = UnixStream::pair().unwrap();
    (
        LaunchChannel::new(first).unwrap(),
        LaunchChannel::new(second).unwrap(),
    )
}
fn raw(value: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).unwrap();
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend(body);
    bytes
}
fn custody(root: &std::path::Path, expected: &OwnerCredential) -> PackageVersionCustody {
    use std::os::unix::fs::OpenOptionsExt;
    let lease = std::fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(root.join("version.lock"))
        .unwrap();
    crate::local_process_owner::package_test_support::custody_fixture(
        lease,
        &root.join("custody.json"),
        expected,
        BINDING,
    )
}

#[test]
fn package_lease_keeps_both_handles_cloexec_and_retains_the_private_channel() {
    let root = tempfile::tempdir().unwrap();
    let (sender, receiver) = pair();
    let expected = credential();
    let sender = sender
        .send_package_lease(Some(custody(root.path(), &expected)), &expected, deadline())
        .unwrap();
    let (received, receiver) = receiver
        .receive_package_lease(&expected, BINDING, deadline())
        .unwrap();
    let handles = received.unwrap().into_process_handles().unwrap();
    for file in &handles {
        assert!(fcntl_getfd(file).unwrap().contains(FdFlags::CLOEXEC));
    }
    sender.send_control(b"after_lease", deadline()).unwrap();
    assert_eq!(
        receiver.receive_control(deadline()).unwrap(),
        b"after_lease"
    );
}

#[test]
fn package_lease_requires_an_explicit_empty_transfer_when_absent() {
    let (sender, receiver) = pair();
    let expected = credential();
    let _sender = sender
        .send_package_lease(None, &expected, deadline())
        .unwrap();
    assert!(receiver
        .receive_package_lease(&expected, BINDING, deadline())
        .unwrap()
        .0
        .is_none());
}

#[test]
fn package_lease_receiver_refuses_readonly_handles_without_mutating_flags() {
    let root = tempfile::tempdir().unwrap();
    let expected = credential();
    let [lease, record] = custody(root.path(), &expected)
        .into_process_handles()
        .unwrap();
    drop(lease);
    let lease = File::open(root.path().join("version.lock")).unwrap();
    let observer = lease.try_clone().unwrap();
    let before = unsafe { libc::fcntl(observer.as_raw_fd(), libc::F_GETFL) };
    let (sender, receiver) = UnixStream::pair().unwrap();
    let receiver = LaunchChannel::new(receiver).unwrap();
    let value = serde_json::to_value(PackageLeaseFrame {
        schema_version: 2,
        credential: expected.clone(),
        has_lease: true,
    })
    .unwrap();
    super::super::tests::raw_send(&sender, &raw(&value), &[&lease, &record]);
    drop(sender);
    assert!(matches!(
        receiver.receive_package_lease(&expected, BINDING, deadline()),
        Err(LaunchChannelFailure::InvalidDescriptor)
    ));
    assert_eq!(
        unsafe { libc::fcntl(observer.as_raw_fd(), libc::F_GETFL) },
        before
    );
}

#[test]
fn package_lease_rejects_wrong_binding_schema_unknown_fields_and_handle_counts() {
    let expected = credential();
    let valid = serde_json::to_value(PackageLeaseFrame {
        schema_version: 2,
        credential: expected.clone(),
        has_lease: false,
    })
    .unwrap();
    for case in [
        "schema",
        "legacy_schema",
        "credential",
        "unknown",
        "missing_handle",
        "extra_handle",
    ] {
        let (sender, receiver) = UnixStream::pair().unwrap();
        let receiver = LaunchChannel::new(receiver).unwrap();
        let mut value = valid.clone();
        match case {
            "schema" => value["schema_version"] = serde_json::json!(3),
            "legacy_schema" => value["schema_version"] = serde_json::json!(1),
            "credential" => value["credential"] = serde_json::to_value(credential()).unwrap(),
            "unknown" => value["path"] = serde_json::json!("not_admitted"),
            "missing_handle" => value["has_lease"] = serde_json::json!(true),
            "extra_handle" => {}
            _ => unreachable!(),
        }
        let file = tempfile::tempfile().unwrap();
        let files = if case == "extra_handle" {
            vec![&file]
        } else {
            vec![]
        };
        super::super::tests::raw_send(&sender, &raw(&value), &files);
        drop(sender);
        assert!(
            receiver
                .receive_package_lease(&expected, BINDING, deadline())
                .is_err(),
            "{case}"
        );
    }
}

#[test]
fn durable_custody_cannot_cross_owner_or_execution_bindings() {
    for wrong_owner in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (sender, receiver) = pair();
        let expected = credential();
        let lease = custody(root.path(), &expected);
        if wrong_owner {
            assert!(matches!(
                sender.send_package_lease(Some(lease), &credential(), deadline()),
                Err(LaunchChannelFailure::CredentialMismatch)
            ));
        } else {
            let _sender = sender
                .send_package_lease(Some(lease), &expected, deadline())
                .unwrap();
            assert!(matches!(
                receiver.receive_package_lease(&expected, [84; 32], deadline()),
                Err(LaunchChannelFailure::InvalidDescriptor)
            ));
        }
    }
}

#[test]
fn released_custody_cannot_be_transferred_again() {
    let root = tempfile::tempdir().unwrap();
    let (sender, _receiver) = pair();
    let expected = credential();
    let lease = custody(root.path(), &expected);
    lease.release().unwrap();
    assert!(matches!(
        sender.send_package_lease(Some(lease), &expected, deadline()),
        Err(LaunchChannelFailure::InvalidDescriptor)
    ));
}

#[test]
fn package_lease_rejects_non_regular_data_and_closes_received_duplicates() {
    let expected = credential();
    let (sender, receiver) = UnixStream::pair().unwrap();
    let receiver = LaunchChannel::new(receiver).unwrap();
    let (input, output) = rustix::pipe::pipe().unwrap();
    let mut input = File::from(input);
    let output = File::from(output);
    let record = tempfile::tempfile().unwrap();
    let value = serde_json::to_value(PackageLeaseFrame {
        schema_version: 2,
        credential: expected.clone(),
        has_lease: true,
    })
    .unwrap();
    super::super::tests::raw_send(&sender, &raw(&value), &[&output, &record]);
    drop(output);
    drop(sender);
    assert!(matches!(
        receiver.receive_package_lease(&expected, BINDING, deadline()),
        Err(LaunchChannelFailure::InvalidDescriptor)
    ));
    assert_eq!(input.read(&mut [0]).unwrap(), 0);
}

#[test]
fn package_lease_truncated_body_closes_received_duplicates() {
    let expected = credential();
    let (sender, receiver) = UnixStream::pair().unwrap();
    let receiver = LaunchChannel::new(receiver).unwrap();
    let (input, output) = rustix::pipe::pipe().unwrap();
    let mut input = File::from(input);
    let output = File::from(output);
    let mut header = MAGIC.to_vec();
    header.extend_from_slice(&100_u32.to_be_bytes());
    super::super::tests::raw_send(&sender, &header, &[&output]);
    drop(output);
    drop(sender);
    assert!(matches!(
        receiver.receive_package_lease(&expected, BINDING, deadline()),
        Err(LaunchChannelFailure::TransportUnavailable)
    ));
    assert_eq!(input.read(&mut [0]).unwrap(), 0);
}

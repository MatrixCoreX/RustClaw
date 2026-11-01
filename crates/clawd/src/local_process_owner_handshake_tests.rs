use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(target_os = "linux")]
use super::super::owner_host_handshake::HostReadyStart;
use super::super::owner_store::tests::{binding, credential, store};
use super::*;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("hs-{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(8)
}
fn pair() -> (LaunchChannel, LaunchChannel) {
    let (first, second) = UnixStream::pair().unwrap();
    (
        LaunchChannel::new(first).unwrap(),
        LaunchChannel::new(second).unwrap(),
    )
}

#[test]
fn no_commit_publisher_rejects_another_actual_process() {
    use super::super::instance::observe_process;
    use super::super::process_launch::{PayloadIo, ProcessLaunch};
    use crate::process_invocation::ProcessInvocation;
    use std::process::{Command, Stdio};
    let root = Directory::new();
    let store = store(&root.0);
    let mut child = Command::new("/bin/sh")
        .args(["-c", "read -r _ || :"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let other = store
        .credential_for(
            uuid::Uuid::new_v4(),
            observe_process(child.id()).unwrap().birth,
            binding(),
        )
        .unwrap();
    let reference = store.publish(&other, binding()).unwrap();
    let launch = ProcessLaunch::new(
        ProcessInvocation::new("/bin/true"),
        root.0.clone(),
        vec![],
        PayloadIo {
            stdin: None,
            stdout: None,
            stderr: None,
        },
    )
    .unwrap();
    let start = OwnerStart::prepare(
        launch,
        other.job_id,
        other.nonce,
        binding(),
        Duration::from_millis(1),
    )
    .unwrap();
    let (channel, peer) = pair();
    let mut prepared = OwnerPreparedStart::new(start, channel).unwrap();
    // Test-only substitution models a copied creator identity. It is not an
    // actual fork or evidence that untrusted callers can mutate private state.
    prepared.credential = other;
    let evidence = prepared.abandon();
    drop(peer);
    let rejected = store.publish_no_commit(&reference, &evidence).is_err();
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    assert!(rejected, "no_commit_publisher_writer_identity_unchecked");
    assert!(store.load_outcome(&reference).unwrap().is_none());
}
fn send(channel: &LaunchChannel, expected: &OwnerCredential, message: StartupMessage) {
    channel
        .send_control(
            &startup_protocol::encode(expected, binding(), message).unwrap(),
            deadline(),
        )
        .unwrap();
}

#[test]
fn owner_handshake_host_refuses_acknowledgment_instead_of_ready() {
    let expected = credential(uuid::Uuid::new_v4());
    let (host, peer) = pair();
    send(
        &peer,
        &expected,
        StartupMessage::Acknowledgment {
            result: StartupAck::Started {},
        },
    );
    assert!(matches!(
        HostPendingStart::new(host, expected, binding())
            .unwrap()
            .wait_ready(deadline()),
        Err(HandshakeFailure::Protocol(OwnerFailure::InvalidState))
    ));
}

#[test]
fn owner_handshake_wrong_proof_is_rejected_before_any_commit_bytes() {
    let root = Directory::new();
    let expected = credential(uuid::Uuid::new_v4());
    let other = credential(uuid::Uuid::new_v4());
    let wrong = store(&root.0).publish(&other, binding()).unwrap();
    let (host, peer) = pair();
    send(&peer, &expected, StartupMessage::Ready {});
    let ready = HostPendingStart::new(host, expected, binding())
        .unwrap()
        .wait_ready(deadline())
        .unwrap();
    assert!(matches!(
        ready.commit(&wrong, deadline()),
        Err(HandshakeFailure::Protocol(OwnerFailure::CredentialMismatch))
    ));
    assert!(matches!(
        peer.receive_control(deadline()),
        Err(LaunchChannelFailure::TransportUnavailable)
    ));
}

#[test]
fn owner_handshake_commit_send_failure_is_uncertain_not_a_safe_spawn_retry() {
    let root = Directory::new();
    let expected = credential(uuid::Uuid::new_v4());
    let reference = store(&root.0).publish(&expected, binding()).unwrap();
    let (host, peer) = pair();
    send(&peer, &expected, StartupMessage::Ready {});
    let ready = HostPendingStart::new(host, expected, binding())
        .unwrap()
        .wait_ready(deadline())
        .unwrap();
    drop(peer);
    let committed = ready.commit(&reference, deadline()).unwrap();
    assert!(matches!(
        committed.wait_started(deadline()),
        CommitObservation::Uncertain(HandshakeFailure::Transport(_))
    ));
}

#[test]
fn owner_handshake_commit_ack_timeout_preserves_uncertainty() {
    let root = Directory::new();
    let expected = credential(uuid::Uuid::new_v4());
    let reference = store(&root.0).publish(&expected, binding()).unwrap();
    let (host, peer) = pair();
    send(&peer, &expected, StartupMessage::Ready {});
    let ready = HostPendingStart::new(host, expected, binding())
        .unwrap()
        .wait_ready(deadline())
        .unwrap();
    let committed = ready.commit(&reference, deadline()).unwrap();
    assert!(matches!(
        committed.wait_started(Instant::now() + Duration::from_millis(30)),
        CommitObservation::Uncertain(HandshakeFailure::Transport(
            LaunchChannelFailure::StartupTimeout
        ))
    ));
}

#[test]
fn owner_handshake_control_channel_rejects_and_closes_ancillary_rights() {
    let expected = credential(uuid::Uuid::new_v4());
    let bytes = startup_protocol::encode(&expected, binding(), StartupMessage::Ready {}).unwrap();
    let (sender, receiver) = UnixStream::pair().unwrap();
    let receiver = LaunchChannel::new(receiver).unwrap();
    let (input, output) = rustix::pipe::pipe().unwrap();
    rustix::io::ioctl_fionbio(&input, true).unwrap();
    let mut input = File::from(input);
    let output = File::from(output);
    let mut frame = b"OST1".to_vec();
    frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    frame.extend(bytes);
    super::super::launch_channel::tests::raw_send(&sender, &frame, &[&output]);
    drop(output);
    drop(sender);
    assert!(matches!(
        receiver.receive_control(deadline()),
        Err(LaunchChannelFailure::InvalidDescriptor)
    ));
    assert_eq!(input.read(&mut [0; 1]).unwrap(), 0);
}

#[test]
fn owner_handshake_control_headers_refuse_invalid_magic_and_excess_lengths() {
    for kind in 0..3 {
        let (sender, receiver) = UnixStream::pair().unwrap();
        let receiver = LaunchChannel::new(receiver).unwrap();
        let mut frame = b"OST1".to_vec();
        let length = if kind == 1 {
            0
        } else {
            super::super::owner_protocol::MAX_CONTROL_BYTES as u32 + 1
        };
        frame.extend_from_slice(&length.to_be_bytes());
        if kind == 0 {
            frame[0] = b'?';
        }
        super::super::launch_channel::tests::raw_send(&sender, &frame, &[]);
        assert!(receiver.receive_control(deadline()).is_err());
    }
}

#[cfg(target_os = "linux")]
struct Fixture {
    root: Directory,
    child: Option<std::process::Child>,
    host: Option<LaunchChannel>,
    raw: Option<UnixStream>,
    expected: OwnerCredential,
    reference_store: OwnerReferenceStore,
}

#[cfg(target_os = "linux")]
impl Fixture {
    fn new(case: &str) -> Self {
        use super::super::process_launch::{PayloadIo, ProcessLaunch};
        use crate::process_invocation::ProcessInvocation;
        let root = Directory::new();
        let job = uuid::Uuid::new_v4();
        let reference_store = reference_store(case, &root.0, true);
        let mut invocation = ProcessInvocation::new(if case == "handshake_spawn_failure" {
            root.0.join("missing-program").into_os_string()
        } else {
            "/bin/sh".into()
        });
        invocation.arg("-c").arg("printf x >> effect; i=0; while [ ! -e release ]; do i=$((i+1)); [ \"$i\" -le 500 ] || exit 3; sleep 0.01; done");
        let launch = ProcessLaunch::new(
            invocation,
            root.0.clone(),
            vec![],
            PayloadIo {
                stdin: None,
                stdout: None,
                stderr: None,
            },
        )
        .unwrap();
        let (child, peer) = super::super::launch_channel::tests::spawn_fixture(
            case,
            &root.0,
            job,
            Some(reference_store.authority_fingerprint().unwrap()),
        );
        let raw = peer.try_clone().unwrap();
        let expected = reference_store
            .credential_for(
                job,
                super::super::instance::observe_process(child.id())
                    .unwrap()
                    .birth,
                binding(),
            )
            .unwrap();
        let host = LaunchChannel::new(peer)
            .unwrap()
            .send(launch, expected.clone(), deadline())
            .unwrap();
        Self {
            root,
            child: Some(child),
            host: Some(host),
            raw: Some(raw),
            expected,
            reference_store,
        }
    }

    fn ready(&mut self) -> HostReadyStart {
        HostPendingStart::new(self.host.take().unwrap(), self.expected.clone(), binding())
            .unwrap()
            .wait_ready(deadline())
            .unwrap()
    }

    fn reference(&self) -> DurableOwnerReference {
        self.reference_store
            .publish(&self.expected, binding())
            .unwrap()
    }

    fn disconnect(&mut self) {
        if let Some(raw) = self.raw.take() {
            let _ = raw.shutdown(std::net::Shutdown::Both);
        }
        drop(self.host.take());
    }

    fn wait_for(&self, name: &str) {
        let end = deadline();
        while !self.root.0.join(name).exists() {
            assert!(Instant::now() < end, "fixture barrier did not arrive");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn finish(&mut self) {
        self.disconnect();
        fs::write(self.root.0.join("release"), b"done").unwrap();
        let result = self.child.take().unwrap().wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        println!("{}", String::from_utf8_lossy(&result.stdout));
    }
}

#[cfg(target_os = "linux")]
impl Drop for Fixture {
    fn drop(&mut self) {
        self.disconnect();
        let _ = fs::write(self.root.0.join("release"), b"done");
        if let Some(mut child) = self.child.take() {
            let end = Instant::now() + Duration::from_secs(2);
            while child.try_wait().unwrap().is_none() && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(2));
            }
            if child.try_wait().unwrap().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owner_handshake_actual_ready_has_no_effect_until_exact_durable_commit() {
    for case in ["handshake_started", "handshake_persistent_authority"] {
        let mut fixture = Fixture::new(case);
        let ready = fixture.ready();
        assert!(!fixture.root.0.join("effect").exists());
        let reference = fixture.reference();
        let committed = ready.commit(&reference, deadline()).unwrap();
        assert_eq!(
            committed.wait_started(deadline()),
            CommitObservation::Acknowledged(StartupAck::Started {})
        );
        fixture.wait_for("attempted");
        fixture.finish();
        assert_eq!(fs::read(fixture.root.0.join("effect")).unwrap(), b"x");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owner_handshake_actual_disconnect_before_ready_or_commit_never_executes() {
    for case in ["handshake_before_ready", "handshake_before_commit"] {
        let mut fixture = Fixture::new(case);
        if case == "handshake_before_commit" {
            let ready = fixture.ready();
            let _reference = fixture.reference();
            drop(ready);
        }
        fixture.disconnect();
        fixture.finish();
        assert!(!fixture.root.0.join("effect").exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owner_handshake_actual_lost_ack_does_not_drop_or_repeat_the_running_scope() {
    let mut fixture = Fixture::new("handshake_lost_ack");
    let ready = fixture.ready();
    let reference = fixture.reference();
    fixture
        .raw
        .as_ref()
        .unwrap()
        .shutdown(std::net::Shutdown::Read)
        .unwrap();
    let committed = ready.commit(&reference, deadline()).unwrap();
    assert!(matches!(
        committed.wait_started(deadline()),
        CommitObservation::Uncertain(_)
    ));
    fixture.wait_for("attempted");
    fixture.wait_for("effect");
    assert_eq!(fs::read(fixture.root.0.join("effect")).unwrap(), b"x");
    fixture.finish();
    assert_eq!(fs::read(fixture.root.0.join("effect")).unwrap(), b"x");
}

#[cfg(target_os = "linux")]
#[test]
fn owner_handshake_actual_missing_reference_or_invalid_commit_cannot_start() {
    for case in [
        "handshake_missing_reference",
        "handshake_bad_binding",
        "handshake_bad_phase",
    ] {
        let mut fixture = Fixture::new(case);
        let ready = fixture.ready();
        let channel =
            LaunchChannel::new(fixture.raw.as_ref().unwrap().try_clone().unwrap()).unwrap();
        let digest = if case == "handshake_bad_binding" {
            [8; 32]
        } else {
            binding()
        };
        let message = if case == "handshake_bad_phase" {
            StartupMessage::Ready {}
        } else {
            StartupMessage::Commit {}
        };
        channel
            .send_control(
                &startup_protocol::encode(&fixture.expected, digest, message).unwrap(),
                deadline(),
            )
            .unwrap();
        if case == "handshake_missing_reference" {
            let bytes = channel.receive_control(deadline()).unwrap();
            assert!(
                startup_protocol::decode(&bytes, &fixture.expected, &binding()).unwrap()
                    == StartupMessage::Acknowledgment {
                        result: StartupAck::RejectedBeforeSpawn {
                            reason: StartupRejection::MissingReference {}
                        }
                    }
            );
        }
        drop(channel);
        drop(ready);
        fixture.finish();
        assert!(!fixture.root.0.join("effect").exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owner_handshake_actual_tampered_reference_rejects_commit_without_replacing_the_claim() {
    use super::super::owner_reference::OwnerReferenceFailure;

    let mut fixture = Fixture::new("handshake_corrupt_reference");
    let ready = fixture.ready();
    let reference = fixture.reference();
    let entries = fs::read_dir(&fixture.root.0)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    let path = &entries[0];
    let mut signed: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    signed["reference"]["binding_digest"] = serde_json::to_value([8_u8; 32]).unwrap();
    let corrupted = serde_json::to_vec(&signed).unwrap();
    fs::write(path, &corrupted).unwrap();

    assert_eq!(
        ready
            .commit(&reference, deadline())
            .unwrap()
            .wait_started(deadline()),
        CommitObservation::Acknowledged(StartupAck::RejectedBeforeSpawn {
            reason: StartupRejection::Reference {
                error_code: OwnerReferenceFailure::AuthenticationFailed
            }
        })
    );
    fixture.finish();
    assert!(!fixture.root.0.join("effect").exists());
    assert_eq!(fs::read(path).unwrap(), corrupted);
    assert!(matches!(
        store(&fixture.root.0).load(fixture.expected.job_id),
        Err(OwnerReferenceFailure::AuthenticationFailed)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn owner_handshake_actual_spawn_failure_is_not_mislabelled_as_precommit_rejection() {
    let mut fixture = Fixture::new("handshake_spawn_failure");
    let ready = fixture.ready();
    let reference = fixture.reference();
    assert_eq!(
        ready
            .commit(&reference, deadline())
            .unwrap()
            .wait_started(deadline()),
        CommitObservation::Acknowledged(StartupAck::AttemptFailed {
            error_code: OwnerFailure::SpawnFailed
        })
    );
    fixture.finish();
    assert!(!fixture.root.0.join("effect").exists());
}

#[cfg(target_os = "linux")]
pub(crate) fn run_case(case: &str) {
    use super::super::owner_protocol::OwnerPhase;
    use std::os::fd::FromRawFd;
    let root = PathBuf::from(std::env::var_os("APP_OWNER_REFERENCE_TEST_ROOT").unwrap());
    let job =
        uuid::Uuid::parse_str(&std::env::var("APP_OWNER_REFERENCE_TEST_JOB").unwrap()).unwrap();
    let reference_store = reference_store(case, &root, false);
    let expected = reference_store
        .credential_for(
            job,
            super::super::instance::observe_process(std::process::id())
                .unwrap()
                .birth,
            binding(),
        )
        .unwrap();
    let socket = unsafe { UnixStream::from_raw_fd(198) };
    let (launch, channel) = LaunchChannel::new(socket)
        .unwrap()
        .receive(&expected, deadline())
        .unwrap();
    let start = OwnerStart::prepare(
        launch,
        job,
        expected.nonce,
        binding(),
        Duration::from_secs(1),
    )
    .unwrap();
    let owner = OwnerPreparedStart::new(start, channel).unwrap();
    let outcome = unsafe { owner.wait_commit(&reference_store, deadline()) };
    match outcome {
        OwnerTransfer::NotCommitted { error, evidence } => {
            assert!(evidence.observed() == (&expected, &binding()));
            assert!(matches!(
                case,
                "handshake_before_ready"
                    | "handshake_before_commit"
                    | "handshake_bad_binding"
                    | "handshake_bad_phase"
            ));
            assert!(matches!(
                error,
                HandshakeFailure::Transport(_) | HandshakeFailure::Protocol(_)
            ));
        }
        OwnerTransfer::Rejected {
            reason,
            acknowledgment,
            evidence,
        } => {
            assert!(evidence.observed() == (&expected, &binding()));
            let expected = match case {
                "handshake_missing_reference" => StartupRejection::MissingReference {},
                "handshake_corrupt_reference" => StartupRejection::Reference {
                    error_code:
                        super::super::owner_reference::OwnerReferenceFailure::AuthenticationFailed,
                },
                _ => panic!("unexpected pre-spawn rejection"),
            };
            assert_eq!(reason, expected);
            acknowledgment.unwrap();
        }
        OwnerTransfer::Attempted {
            mut start,
            reference,
            result,
            acknowledgment,
        } => {
            assert!(reference.authorizes(&start.credential(), &binding()));
            if case == "handshake_spawn_failure" {
                assert_eq!(
                    result,
                    StartupAck::AttemptFailed {
                        error_code: OwnerFailure::SpawnFailed
                    }
                );
                acknowledgment.unwrap();
                assert!(matches!(start.runtime(), Err(OwnerFailure::SpawnFailed)));
                return;
            }
            assert_eq!(result, StartupAck::Started {});
            if case == "handshake_lost_ack" {
                assert!(acknowledgment.is_err());
            } else {
                acknowledgment.unwrap();
            }
            assert_eq!(
                start.runtime().unwrap().tick().unwrap().phase,
                OwnerPhase::Running
            );
            fs::write(root.join("attempted"), b"live").unwrap();
            let end = deadline();
            loop {
                let snapshot = start.runtime().unwrap().tick().unwrap();
                if snapshot.phase == OwnerPhase::Terminal {
                    assert_eq!(snapshot.root_exit.unwrap().code, Some(0));
                    break;
                }
                assert!(Instant::now() < end);
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn reference_store(case: &str, root: &std::path::Path, initialize: bool) -> OwnerReferenceStore {
    if case == "handshake_persistent_authority" {
        let broker = super::super::owner_authority::OwnerAuthorityBroker::from_admitted_directory(
            File::open(root).unwrap(),
        )
        .unwrap();
        if initialize {
            broker.initialize_new().unwrap()
        } else {
            let bytes: Vec<u8> = serde_json::from_str(
                &std::env::var("APP_OWNER_AUTHORITY_TEST_FINGERPRINT").unwrap(),
            )
            .unwrap();
            let expected =
                super::super::owner_reference::AuthorityFingerprint::from_slice(&bytes).unwrap();
            broker.load_expected(&expected).unwrap()
        }
    } else {
        store(root)
    }
}

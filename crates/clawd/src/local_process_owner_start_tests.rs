use std::fs;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::io::Write;
#[cfg(target_os = "linux")]
use std::time::Instant;

use super::super::owner_store::tests::{binding, credential, store};
use super::super::process_launch::PayloadIo;
use super::*;
use crate::process_invocation::ProcessInvocation;

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("owner-start-{}", Uuid::new_v4().simple()));
        fs::create_dir(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn null_io() -> PayloadIo {
    PayloadIo {
        stdin: None,
        stdout: None,
        stderr: None,
    }
}

fn launch(
    program: impl AsRef<std::ffi::OsStr>,
    args: &[&std::ffi::OsStr],
    root: &Directory,
    io: PayloadIo,
) -> ProcessLaunch {
    let mut invocation = ProcessInvocation::new(program);
    for arg in args {
        invocation.arg(arg);
    }
    ProcessLaunch::new(invocation, root.0.clone(), vec![], io).unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn owner_start_lost_ack_and_repeated_commit_never_repeat_a_live_payload() {
    super::super::owner_scope::tests::run_isolated("start_replay", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_start_failed_spawn_stays_failed_even_if_the_program_becomes_available() {
    super::super::owner_scope::tests::run_isolated("start_failed", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_start_refuses_a_changed_owner_birth_before_effects() {
    super::super::owner_scope::tests::run_isolated("start_binding", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_start_requires_the_exact_durable_reference_before_any_payload_effect() {
    super::super::owner_scope::tests::run_isolated("start_proof_refusal", None);
}

#[test]
fn owner_start_preparation_and_abandonment_have_no_payload_effect() {
    let root = Directory::new();
    let marker = root.0.join("unexpected");
    let launch = launch(
        "/bin/sh",
        &[
            "-c".as_ref(),
            "printf unexpected > \"$1\"".as_ref(),
            "fixture".as_ref(),
            marker.as_os_str(),
        ],
        &root,
        null_io(),
    );
    let mut start = OwnerStart::prepare(
        launch,
        Uuid::new_v4(),
        Uuid::new_v4(),
        binding(),
        Duration::ZERO,
    )
    .unwrap();
    assert!(matches!(start.runtime(), Err(OwnerFailure::InvalidState)));
    assert!(start.startup_failure_evidence().is_none());
    assert!(!marker.exists());
    drop(start);
    assert!(!marker.exists());
}

#[test]
fn owner_start_invalid_job_or_nonce_never_launches() {
    let root = Directory::new();
    for (job, nonce) in [(Uuid::nil(), Uuid::new_v4()), (Uuid::new_v4(), Uuid::nil())] {
        let launch = launch("/bin/sh", &[], &root, null_io());
        assert!(matches!(
            OwnerStart::prepare(launch, job, nonce, binding(), Duration::ZERO),
            Err(OwnerFailure::InvalidCredential)
        ));
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn run_case(case: &str) {
    use super::super::owner_protocol::{OwnerCompletion, OwnerPhase};
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;

    let root = Directory::new();
    let marker = root.0.join("counter");
    let program = root.0.join("program");
    let stdout = root.0.join("stdout");
    let (input, mut writer) = UnixStream::pair().unwrap();
    let input = File::from(OwnedFd::from(input));
    let source = launch(
        if case == "start_failed" {
            program.as_os_str()
        } else {
            "/bin/sh".as_ref()
        },
        &[
            "-c".as_ref(),
            "printf x >> \"$1\"; read -r value; printf '%s' \"$value\"".as_ref(),
            "fixture".as_ref(),
            marker.as_os_str(),
        ],
        &root,
        PayloadIo {
            stdin: Some(input),
            stdout: Some(File::create(&stdout).unwrap()),
            stderr: None,
        },
    );
    let expected = credential(Uuid::new_v4());
    let reference = store(&root.0).publish(&expected, binding()).unwrap();
    let mut start = OwnerStart::prepare(
        source,
        expected.job_id,
        expected.nonce,
        binding(),
        Duration::from_secs(1),
    )
    .unwrap();
    if case == "start_proof_refusal" {
        let other = credential(Uuid::new_v4());
        let other_reference = store(&root.0).publish(&other, binding()).unwrap();
        assert!(matches!(
            unsafe { start.commit(&other_reference) },
            Err(OwnerFailure::CredentialMismatch)
        ));
        assert!(!marker.exists());
        start.binding_digest = [8; 32];
        assert!(matches!(
            unsafe { start.commit(&reference) },
            Err(OwnerFailure::CredentialMismatch)
        ));
        assert!(!marker.exists());
        start.binding_digest = binding();
    }
    match case {
        "start_binding" => {
            start.credential.owner =
                super::super::instance::observe_process(unsafe { libc::getppid() } as u32)
                    .unwrap()
                    .birth;
            for _ in 0..3 {
                assert!(matches!(
                    unsafe { start.commit(&reference) },
                    Err(OwnerFailure::CredentialMismatch)
                ));
            }
            assert!(!marker.exists());
        }
        "start_failed" => {
            assert!(matches!(
                unsafe { start.commit(&reference) },
                Err(OwnerFailure::SpawnFailed)
            ));
            let evidence = start.startup_failure_evidence().unwrap();
            let authority = store(&root.0);
            let receipt = authority
                .publish_startup_failure(&reference, &evidence)
                .unwrap();
            assert!(receipt.authorizes(&expected, &binding()));
            assert_eq!(receipt.error_code(), OwnerFailure::SpawnFailed);
            assert!(authority
                .publish_startup_failure(&reference, &evidence)
                .is_ok());
            let other = credential(Uuid::new_v4());
            let wrong = authority.publish(&other, binding()).unwrap();
            assert!(matches!(
                authority.publish_startup_failure(&wrong, &evidence),
                Err(super::super::owner_reference::OwnerReferenceFailure::AuthenticationFailed)
            ));
            fs::write(&program, b"#!/bin/sh\nprintf unexpected > counter\n").unwrap();
            fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
            for _ in 0..3 {
                assert!(matches!(
                    unsafe { start.commit(&reference) },
                    Err(OwnerFailure::SpawnFailed)
                ));
            }
            assert!(!marker.exists());
            assert!(matches!(start.runtime(), Err(OwnerFailure::SpawnFailed)));
        }
        "start_replay" | "start_proof_refusal" => {
            let ready = start.credential();
            // Discard the first acknowledgement; the owner still retains work.
            drop(unsafe { start.commit(&reference) }.unwrap());
            assert!(start.startup_failure_evidence().is_none());
            let deadline = Instant::now() + Duration::from_secs(5);
            while !marker.exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(
                start.runtime().unwrap().tick().unwrap().phase,
                OwnerPhase::Running
            );
            for _ in 0..3 {
                assert!(unsafe { start.commit(&reference) }.unwrap() == ready);
            }
            assert_eq!(fs::read(&marker).unwrap(), b"x");
            writer.write_all(b"done\n").unwrap();
            drop(writer);
            let done = loop {
                let snapshot = start.runtime().unwrap().tick().unwrap();
                if snapshot.phase == OwnerPhase::Terminal {
                    break snapshot;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            };
            assert_eq!(done.completion, Some(OwnerCompletion::Completed));
            assert_eq!(done.root_exit.unwrap().code, Some(0));
            assert!(unsafe { start.commit(&reference) }.unwrap() == ready);
            assert_eq!(fs::read(marker).unwrap(), b"x");
            assert_eq!(fs::read(stdout).unwrap(), b"done");
        }
        _ => panic!("unknown owner start fixture"),
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn owner_start_unsupported_scope_is_cached_without_launching_the_program() {
    let root = Directory::new();
    let marker = root.0.join("unexpected");
    let launch = launch(
        "/bin/sh",
        &[
            "-c".as_ref(),
            "printf unexpected > \"$1\"".as_ref(),
            "fixture".as_ref(),
            marker.as_os_str(),
        ],
        &root,
        null_io(),
    );
    let expected = credential(Uuid::new_v4());
    let reference = store(&root.0).publish(&expected, binding()).unwrap();
    let mut start = OwnerStart::prepare(
        launch,
        expected.job_id,
        expected.nonce,
        binding(),
        Duration::ZERO,
    )
    .unwrap();
    for _ in 0..3 {
        assert!(matches!(
            unsafe { start.commit(&reference) },
            Err(OwnerFailure::Unsupported)
        ));
    }
    assert!(!marker.exists());
    assert!(matches!(start.runtime(), Err(OwnerFailure::Unsupported)));
}

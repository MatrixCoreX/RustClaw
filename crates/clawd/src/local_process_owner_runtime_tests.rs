#[cfg(target_os = "linux")]
use std::{process::Stdio, time::Instant};

use super::super::owner_protocol::OwnerOutcome;
#[cfg(target_os = "linux")]
use super::super::owner_protocol::{OwnerCompletion, OwnerPhase};
use super::*;

#[cfg(target_os = "linux")]
fn isolated(case: &str) {
    super::super::owner_scope::tests::run_isolated(case, None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_runtime_inspects_then_observes_actual_completion() {
    isolated("runtime_inspect");
}

#[cfg(target_os = "linux")]
#[test]
fn owner_runtime_cancel_replay_never_substitutes_acknowledgement_for_exit() {
    isolated("runtime_cancel");
}

#[cfg(target_os = "linux")]
#[test]
fn owner_runtime_late_cancel_does_not_relabel_completed_work() {
    isolated("runtime_late_cancel");
}

#[cfg(target_os = "linux")]
#[test]
fn owner_runtime_refuses_an_unrelated_credential_before_cancelling() {
    isolated("runtime_credential");
}

#[cfg(target_os = "linux")]
#[test]
fn owner_runtime_lost_child_reference_never_becomes_terminal_success() {
    isolated("runtime_lost");
}

#[cfg(target_os = "linux")]
#[test]
fn owner_runtime_invalid_launch_credential_never_starts_a_payload() {
    isolated("runtime_invalid");
}

#[cfg(target_os = "linux")]
pub(crate) fn run_case(case: &str) {
    let mut command = Command::new("/bin/sh");
    command
        .args([
            "-c",
            if case == "runtime_late_cancel" {
                "exit 7"
            } else if case == "runtime_lost" {
                "exit 3"
            } else {
                "read -r value"
            },
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if case == "runtime_invalid" {
        let fixture =
            std::env::temp_dir().join(format!("agent-owner-invalid-{}", Uuid::new_v4().simple()));
        command = Command::new("/bin/sh");
        command
            .args(["-c", "printf unexpected > \"$1\"", "fixture"])
            .arg(&fixture);
        assert!(matches!(
            unsafe {
                OwnerRuntime::spawn(&mut command, Uuid::new_v4(), Uuid::nil(), Duration::ZERO)
            },
            Err(OwnerFailure::InvalidCredential)
        ));
        assert!(!fixture.exists());
        return;
    }
    let mut owner = unsafe {
        OwnerRuntime::spawn(
            &mut command,
            Uuid::new_v4(),
            Uuid::new_v4(),
            Duration::from_secs(5),
        )
    }
    .unwrap();
    let input = owner.take_stdin();
    let make_request = |owner: &OwnerRuntime, request_id, action| OwnerRequest {
        schema_version: 1,
        credential: owner.credential(),
        request_id,
        action,
    };
    match case {
        "runtime_lost" => {
            let deadline = Instant::now() + Duration::from_secs(5);
            let pid = loop {
                let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
                assert_eq!(
                    unsafe {
                        libc::waitid(
                            libc::P_ALL,
                            0,
                            &mut info,
                            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT | libc::__WALL,
                        )
                    },
                    0
                );
                let pid = unsafe { info.si_pid() };
                if pid > 1 {
                    break pid;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            };
            let mut status = 0;
            assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
            let request = make_request(&owner, 1, OwnerAction::Inspect);
            let failed = owner.handle(&request).unwrap();
            assert_eq!(
                failed.outcome,
                OwnerOutcome::Error {
                    error_code: OwnerFailure::OwnershipLost
                }
            );
            assert_eq!(owner.handle(&request), Ok(failed));
            let cancel = make_request(&owner, 2, OwnerAction::Cancel);
            assert_eq!(
                owner.handle(&cancel).unwrap().outcome,
                OwnerOutcome::Error {
                    error_code: OwnerFailure::OwnershipLost
                }
            );
        }
        "runtime_late_cancel" => {
            let done = wait_terminal(&mut owner);
            let request = make_request(&owner, 1, OwnerAction::Cancel);
            assert_eq!(
                owner.handle(&request).unwrap().outcome,
                OwnerOutcome::Ok { snapshot: done }
            );
            assert_eq!(done.completion, Some(OwnerCompletion::Completed));
            assert_eq!(done.root_exit.unwrap().code, Some(7));
        }
        "runtime_credential" => {
            let mut request = make_request(&owner, 1, OwnerAction::Cancel);
            request.credential.nonce = Uuid::new_v4();
            assert_eq!(
                owner.handle(&request),
                Err(OwnerFailure::CredentialMismatch)
            );
            assert_eq!(owner.tick().unwrap().phase, OwnerPhase::Running);
            drop(input);
            assert_eq!(
                wait_terminal(&mut owner).completion,
                Some(OwnerCompletion::Completed)
            );
        }
        "runtime_cancel" => {
            let request = make_request(&owner, 1, OwnerAction::Cancel);
            let ack = owner.handle(&request).unwrap();
            for _ in 0..5 {
                assert_eq!(owner.handle(&request), Ok(ack));
            }
            let end = wait_terminal(&mut owner);
            assert_eq!(end.completion, Some(OwnerCompletion::Cancelled));
            assert_eq!(end.root_exit.unwrap().signal, Some(libc::SIGTERM));
            let inspect = make_request(&owner, 2, OwnerAction::Inspect);
            assert_eq!(
                owner.handle(&inspect).unwrap().outcome,
                OwnerOutcome::Ok { snapshot: end }
            );
        }
        "runtime_inspect" => {
            let request = make_request(&owner, 1, OwnerAction::Inspect);
            let OwnerOutcome::Ok { snapshot } = owner.handle(&request).unwrap().outcome else {
                panic!("inspect failed")
            };
            assert_eq!(snapshot.phase, OwnerPhase::Running);
            drop(input);
            assert_eq!(
                wait_terminal(&mut owner).completion,
                Some(OwnerCompletion::Completed)
            );
        }
        _ => panic!("unknown runtime fixture"),
    }
}

#[cfg(target_os = "linux")]
fn wait_terminal(owner: &mut OwnerRuntime) -> OwnerSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = owner.tick().unwrap();
        if snapshot.phase == OwnerPhase::Terminal {
            return snapshot;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn owner_runtime_unsupported_scope_never_falls_back_to_group_authority() {
    let mut command = Command::new("/usr/bin/true");
    assert!(matches!(
        unsafe {
            OwnerRuntime::spawn(&mut command, Uuid::new_v4(), Uuid::new_v4(), Duration::ZERO)
        },
        Err(OwnerFailure::Unsupported)
    ));
    let credential = OwnerCredential {
        job_id: Uuid::new_v4(),
        nonce: Uuid::new_v4(),
        owner: observe_process(std::process::id()).unwrap().birth,
    };
    let mut owner = OwnerRuntime {
        session: OwnerSession::new(credential).unwrap(),
        scope: OwnedProcessScope {},
    };
    assert!(owner.take_stdin().is_none());
    assert_eq!(owner.tick(), Err(OwnerFailure::Unsupported));
    let request = OwnerRequest {
        schema_version: 1,
        credential: owner.credential(),
        request_id: 1,
        action: OwnerAction::Inspect,
    };
    assert_eq!(
        owner.handle(&request).unwrap().outcome,
        OwnerOutcome::Error {
            error_code: OwnerFailure::Unsupported
        }
    );
}

use super::*;

use super::super::OwnedProcessScope;
use std::process::Stdio;

#[test]
fn scope_root_status_is_recorded_only_for_the_root_child() {
    let exit = ChildExit {
        code: Some(7),
        signal: None,
    };
    assert_eq!(root_exit_after_reap(None, 42, 42, exit), Some(exit));
    assert_eq!(root_exit_after_reap(None, 42, 43, exit), None);
}

#[test]
fn scope_root_status_survives_a_later_adopted_child_reusing_its_pid() {
    let root = ChildExit {
        code: Some(7),
        signal: None,
    };
    for adopted in [
        ChildExit {
            code: Some(3),
            signal: None,
        },
        ChildExit {
            code: None,
            signal: Some(libc::SIGKILL),
        },
    ] {
        assert_eq!(
            root_exit_after_reap(Some(root), 42, 42, adopted),
            Some(root)
        );
        assert_eq!(
            root_exit_after_reap(Some(root), 42, 43, adopted),
            Some(root)
        );
    }
}

pub(crate) fn run_recovery_case(case: &str) {
    let root = std::env::temp_dir().join(format!(
        "agent-scope-recovery-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&root).unwrap();
    let ready = root.join("ready");
    let mut command = Command::new("/bin/sh");
    command
        .args([
            "-c",
            if case == "scope_recovery_reaped" {
                "exit 0"
            } else if case == "scope_recovery_deadline" {
                "trap '' TERM; printf ready > \"$1\"; read -r value"
            } else {
                "read -r value"
            },
            "fixture",
        ])
        .arg(&ready)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // This helper is reached only by a parent-selected isolated owner fixture.
    let mut scope = unsafe {
        OwnedProcessScope::spawn(
            &mut command,
            if case == "scope_recovery_deadline" {
                Duration::from_secs(60)
            } else {
                Duration::ZERO
            },
        )
    }
    .unwrap();
    let _input = scope.take_stdin().unwrap();
    match case {
        "scope_recovery_cancel" => {
            let ack = scope
                .scope
                .cancel_observed(Err(ScopeFailure::ProbeUnavailable))
                .unwrap();
            assert!(ack.cancel_requested);
            let end = wait_complete(&mut scope);
            assert!(end.cancel_requested);
            assert_eq!(end.root_exit.unwrap().signal, Some(libc::SIGKILL));
        }
        "scope_recovery_escalation" => {
            scope.scope.cancellation = Some(Instant::now());
            scope.scope.snapshot.cancel_requested = true;
            assert_eq!(
                scope
                    .scope
                    .poll_observed(Err(ScopeFailure::ProbeUnavailable)),
                Err(ScopeFailure::ProbeUnavailable)
            );
            assert_eq!(
                wait_complete(&mut scope).root_exit.unwrap().signal,
                Some(libc::SIGKILL)
            );
        }
        "scope_recovery_lost" => {
            assert_eq!(
                scope
                    .scope
                    .cancel_observed(Err(ScopeFailure::OwnershipLost)),
                Err(ScopeFailure::OwnershipLost)
            );
            assert!(!scope.scope.snapshot.cancel_requested);
            scope.scope.cancellation = Some(Instant::now());
            scope.scope.snapshot.cancel_requested = true;
            assert_eq!(
                scope.scope.poll_observed(Err(ScopeFailure::OwnershipLost)),
                Err(ScopeFailure::OwnershipLost)
            );
            assert!(scope.scope.sent.is_empty());
            assert_eq!(scope.poll().unwrap().root_exit, None);
        }
        "scope_recovery_reaped" => {
            let pid = scope.scope.root.id() as i32;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
                assert_eq!(
                    unsafe {
                        libc::waitid(
                            libc::P_PID,
                            pid as libc::id_t,
                            &mut info,
                            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                        )
                    },
                    0
                );
                if unsafe { info.si_pid() } == pid {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            let mut status = 0;
            assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
            assert_eq!(
                scope
                    .scope
                    .cancel_observed(Err(ScopeFailure::ProbeUnavailable)),
                Err(ScopeFailure::OwnershipLost)
            );
            assert!(!scope.scope.snapshot.complete);
            assert!(scope.scope.sent.is_empty());
        }
        "scope_recovery_deadline" => {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !std::fs::metadata(&ready).is_ok_and(|metadata| metadata.len() > 0) {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            scope.cancel().unwrap();
            let since = scope.scope.cancellation;
            for _ in 0..5 {
                scope.cancel().unwrap();
                assert_eq!(scope.scope.cancellation, since);
            }
            assert_eq!(
                scope.scope.sent.get(&scope.scope.root.id()),
                Some(&libc::SIGTERM)
            );
        }
        _ => panic!("unknown scope recovery fixture"),
    }
    drop(scope);
    std::fs::remove_dir_all(root).unwrap();
}

fn wait_complete(scope: &mut OwnedProcessScope) -> ScopeSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = scope.poll().unwrap();
        if snapshot.complete {
            return snapshot;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}

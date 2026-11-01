use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::super::super::instance::{observe_process, ProcessProbeFailure};
use super::super::{OwnedProcessScope, ScopeFailure, ScopeSnapshot};

fn command(script: &str) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn spawn(command: &mut Command, grace: Duration) -> OwnedProcessScope {
    // Each caller runs inside the exact ignored fixture in a new process.
    // No child waiter/spawner runs concurrently in that dedicated fixture.
    unsafe { OwnedProcessScope::spawn(command, grace) }.unwrap()
}

fn poll_until(
    scope: &mut OwnedProcessScope,
    ready: impl Fn(ScopeSnapshot) -> bool,
) -> ScopeSnapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = scope.poll().unwrap();
        if ready(status) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "isolated scope did not reach expected state"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub(super) fn run(case: &str) {
    match case {
        "privacy_probe"
        | "privacy_scope_lost"
        | "privacy_scope_unprotected"
        | "privacy_fork"
        | "privacy_descriptor"
        | "privacy_high_descriptor" => super::super::super::owner_privacy::tests::run_case(case),
        "terminal_complete"
        | "terminal_descendant"
        | "terminal_cancel"
        | "terminal_lost"
        | "terminal_concurrent"
        | "terminal_write_failure"
        | "terminal_wrong_proof"
        | "terminal_corrupt" => super::super::super::owner_terminal::store_tests::run_case(case),
        "admission_reserve_exit" | "admission_ready_exit" => {
            super::super::super::owner_admission::tests::run_special_case(case)
        }
        "start_replay" | "start_failed" | "start_binding" | "start_proof_refusal" => {
            super::super::super::owner_start::tests::run_case(case)
        }
        "transport_server" => super::super::super::owner_transport::tests::run_server(),
        "launch_handoff" => super::super::super::launch_channel::tests::run_handoff(),
        "authority_reload" | "authority_write_refused" | "authority_fd_exhausted" => {
            super::super::super::owner_authority::tests::run_special_case(case)
        }
        "owner_ref_spawn" | "owner_ref_frontend" | "owner_ref_detached" => {
            super::super::super::owner_store::tests::run_special_case(case)
        }
        "handshake_started"
        | "handshake_persistent_authority"
        | "handshake_before_ready"
        | "handshake_before_commit"
        | "handshake_lost_ack"
        | "handshake_missing_reference"
        | "handshake_corrupt_reference"
        | "handshake_bad_binding"
        | "handshake_bad_phase"
        | "handshake_spawn_failure" => super::super::super::owner_handshake::tests::run_case(case),
        "scope_recovery_cancel"
        | "scope_recovery_escalation"
        | "scope_recovery_lost"
        | "scope_recovery_reaped"
        | "scope_recovery_deadline" => super::super::linux::tests::run_recovery_case(case),
        "runtime_inspect"
        | "runtime_cancel"
        | "runtime_late_cancel"
        | "runtime_credential"
        | "runtime_lost"
        | "runtime_invalid" => super::super::super::owner_runtime::tests::run_case(case),
        "direct" => direct(),
        "orphan" => orphan(false),
        "nested" => orphan(true),
        "adoption_after_cancel" => adoption_after_cancel(),
        "drop" => drop_remaining(),
        "busy" => busy(),
        "sigchld" => sigchld(),
        "reserved" => reserved(),
        "spawn_failed" => spawn_failed(),
        "no_deadline" => no_deadline(),
        "foreign" => foreign(),
        _ => panic!("unknown isolated scope fixture"),
    }
}

fn direct() {
    let mut scope = spawn(&mut command("exit 7"), Duration::ZERO);
    let done = poll_until(&mut scope, |status| status.complete);
    assert_eq!(done.root_exit.unwrap().code, Some(7));
    assert!(!done.cancel_requested);
    assert_eq!(scope.cancel(), Ok(done));
    for _ in 0..5 {
        assert_eq!(scope.poll(), Ok(done));
    }
}

fn orphan(new_session: bool) {
    let fixture = ScopeFixture::new();
    let mut command = fixture.descendant_command(new_session, false);
    let mut scope = spawn(&mut command, Duration::ZERO);
    let input = scope.take_stdin().unwrap();
    fixture.wait_ready();
    let pending = poll_until(&mut scope, |status| status.root_exit.is_some());
    assert!(!pending.complete);
    assert_eq!(pending.root_exit.unwrap().code, Some(0));
    let pid = fixture.child_pid();
    assert_eq!(observe_process(pid).unwrap().parent_pid, std::process::id());
    if new_session {
        assert_ne!(
            observe_process(pid).unwrap().process_group,
            fixture.root_pid()
        );
        assert!(scope.cancel().is_ok());
    } else {
        drop(input);
    }
    let done = poll_until(&mut scope, |status| status.complete);
    assert_eq!(done.root_exit.unwrap().code, Some(0));
    assert_eq!(done.cancel_requested, new_session);
    assert_eq!(observe_process(pid), Err(ProcessProbeFailure::Missing));
}

fn drop_remaining() {
    let fixture = ScopeFixture::new();
    let mut scope = spawn(&mut fixture.descendant_command(true, false), Duration::ZERO);
    let _input = scope.take_stdin().unwrap();
    fixture.wait_ready();
    poll_until(&mut scope, |status| status.root_exit.is_some());
    let pid = fixture.child_pid();
    drop(scope);
    assert_eq!(observe_process(pid), Err(ProcessProbeFailure::Missing));
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG | libc::__WALL) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

fn adoption_after_cancel() {
    let fixture = ScopeFixture::new();
    let mut scope = spawn(
        &mut fixture.descendant_command(true, true),
        Duration::from_millis(10),
    );
    let _input = scope.take_stdin().unwrap();
    fixture.wait_ready();
    let child = fixture.child_pid();
    let root = fixture.root_pid();
    assert_eq!(scope.poll().unwrap().root_exit, None);
    assert_eq!(observe_process(child).unwrap().parent_pid, root);
    assert_ne!(observe_process(child).unwrap().process_group, root);
    scope.cancel().unwrap();
    let done = poll_until(&mut scope, |status| status.complete);
    assert_eq!(done.root_exit.unwrap().signal, Some(libc::SIGKILL));
    assert!(done.cancel_requested);
    assert_eq!(observe_process(child), Err(ProcessProbeFailure::Missing));
}

fn busy() {
    let mut child = command("read -r value").spawn().unwrap();
    let input = child.stdin.take().unwrap();
    let fixture = ScopeFixture::new();
    let marker = fixture.0.join("unexpected");
    let mut payload = command("printf unexpected > \"$1\"");
    payload.arg("fixture").arg(&marker);
    assert!(matches!(
        unsafe { OwnedProcessScope::spawn(&mut payload, Duration::ZERO) },
        Err(ScopeFailure::OwnerContextInUse)
    ));
    assert!(!marker.exists());
    assert_eq!(child.try_wait().unwrap(), None);
    drop(input);
    assert_eq!(child.wait().unwrap().code(), Some(1));
    let mut enabled = -1_i32;
    assert_eq!(
        unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut enabled, 0, 0, 0) },
        0
    );
    assert_eq!(enabled, 0);
}

fn sigchld() {
    for auto_reap in [false, true] {
        let mut disposition = unsafe { std::mem::zeroed::<libc::sigaction>() };
        disposition.sa_sigaction = if auto_reap {
            libc::SIG_DFL
        } else {
            libc::SIG_IGN
        };
        disposition.sa_flags = if auto_reap { libc::SA_NOCLDWAIT } else { 0 };
        let mut previous = unsafe { std::mem::zeroed::<libc::sigaction>() };
        assert_eq!(
            unsafe { libc::sigaction(libc::SIGCHLD, &disposition, &mut previous) },
            0
        );
        let result = unsafe { OwnedProcessScope::spawn(&mut command("exit 0"), Duration::ZERO) };
        assert_eq!(
            unsafe { libc::sigaction(libc::SIGCHLD, &previous, std::ptr::null_mut()) },
            0
        );
        assert!(matches!(
            result,
            Err(ScopeFailure::InvalidSignalDisposition)
        ));
    }
}

fn reserved() {
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
        0
    );
    assert!(matches!(
        unsafe { OwnedProcessScope::spawn(&mut command("exit 0"), Duration::ZERO) },
        Err(ScopeFailure::OwnerContextInUse)
    ));
}

fn spawn_failed() {
    let fixture = ScopeFixture::new();
    let mut payload = Command::new(fixture.0.join("missing-program"));
    assert!(matches!(
        unsafe { OwnedProcessScope::spawn(&mut payload, Duration::ZERO) },
        Err(ScopeFailure::SpawnFailed)
    ));
    assert!(matches!(
        unsafe { OwnedProcessScope::spawn(&mut command("exit 0"), Duration::ZERO) },
        Err(ScopeFailure::OwnerContextInUse)
    ));
}

fn no_deadline() {
    let mut scope = spawn(&mut command("read -r value"), Duration::ZERO);
    let input = scope.take_stdin().unwrap();
    for _ in 0..20 {
        let status = scope.poll().unwrap();
        assert!(!status.complete);
        assert!(!status.cancel_requested);
        assert_eq!(status.root_exit, None);
    }
    drop(input);
    assert!(!poll_until(&mut scope, |status| status.complete).cancel_requested);
}

fn foreign() {
    let foreign: i32 = std::env::var("APP_PROCESS_SCOPE_TEST_FOREIGN_GROUP")
        .unwrap()
        .parse()
        .unwrap();
    assert!(foreign > 1);
    let mut payload = command("read -r value");
    unsafe {
        payload.pre_exec(move || {
            if libc::setpgid(0, foreign) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let mut scope = spawn(&mut payload, Duration::ZERO);
    let _input = scope.take_stdin().unwrap();
    scope.cancel().unwrap();
    assert!(poll_until(&mut scope, |status| status.complete).cancel_requested);
}

struct ScopeFixture(std::path::PathBuf);

impl ScopeFixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-owner-scope-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn descendant_command(&self, new_session: bool, keep_root: bool) -> Command {
        let prefix = if new_session { "setsid " } else { "" };
        let ignored_term = if keep_root { "trap '' TERM; " } else { "" };
        let tail = if keep_root { "; read -r root_wait" } else { "" };
        let script = format!("exec 3<&0; {ignored_term}printf '%s' \"$$\" > \"$1\"; {prefix}/bin/sh -c 'printf ready > \"$1\"; read -r value' child \"$3\" <&3 & printf '%s' \"$!\" > \"$2\"{tail}");
        let mut payload = command(&script);
        payload
            .arg("fixture")
            .arg(self.0.join("root"))
            .arg(self.0.join("child"))
            .arg(self.0.join("ready"));
        payload
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !std::fs::metadata(self.0.join("ready")).is_ok_and(|metadata| metadata.len() > 0) {
            assert!(Instant::now() < deadline, "escaped child was not ready");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn child_pid(&self) -> u32 {
        std::fs::read_to_string(self.0.join("child"))
            .unwrap()
            .parse()
            .unwrap()
    }
    fn root_pid(&self) -> u32 {
        std::fs::read_to_string(self.0.join("root"))
            .unwrap()
            .parse()
            .unwrap()
    }
}

impl Drop for ScopeFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

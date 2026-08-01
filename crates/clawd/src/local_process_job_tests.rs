use super::{
    maybe_escalate_cancel, process_identity_state, process_loss_is_stable, read_output_delta,
    recover_pending_cancel_escalations, terminate_verified_process_group, ProcessIdentityState,
};

struct TempDirGuard {
    path: std::path::PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent_local_process_job_{prefix}_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("create tempdir");
        Self { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn output_snapshot_uses_durable_monotonic_byte_cursors() {
    let root = TempDirGuard::new("output_cursor");
    let path = root.path().join("stdout");
    let cursor = root.path().join("stdout_cursor");
    std::fs::write(&path, b"0123456789").expect("write output");
    let snapshot = read_output_delta(&path, &cursor, 4);
    assert_eq!(snapshot.text, "0123");
    assert_eq!(snapshot.start_cursor, 0);
    assert_eq!(snapshot.end_cursor, 4);
    assert_eq!(snapshot.total_bytes, 10);
    assert!(snapshot.truncated);
    assert!(!snapshot.cursor_reset);

    let next = read_output_delta(&path, &cursor, 4);
    assert_eq!(next.text, "4567");
    assert_eq!(next.start_cursor, 4);
    assert_eq!(next.end_cursor, 8);
}

#[test]
fn output_cursor_resets_after_stream_truncation() {
    let root = TempDirGuard::new("cursor_reset");
    let path = root.path().join("stderr");
    let cursor = root.path().join("stderr_cursor");
    std::fs::write(&path, b"old output").expect("write old output");
    std::fs::write(&cursor, b"20").expect("write cursor");
    std::fs::write(&path, b"new").expect("truncate output");

    let snapshot = read_output_delta(&path, &cursor, 16);
    assert_eq!(snapshot.text, "new");
    assert_eq!(snapshot.start_cursor, 0);
    assert_eq!(snapshot.end_cursor, 3);
    assert!(snapshot.cursor_reset);
}

#[test]
fn output_delta_never_splits_a_valid_utf8_scalar() {
    let root = TempDirGuard::new("utf8_boundary");
    let path = root.path().join("stdout");
    let cursor = root.path().join("stdout_cursor");
    std::fs::write(&path, "中A").expect("write utf8 output");

    let first = read_output_delta(&path, &cursor, 2);
    assert_eq!(first.text, "中");
    assert_eq!(first.start_cursor, 0);
    assert_eq!(first.end_cursor, 3);
    assert_eq!(first.encoding, "utf-8");

    let second = read_output_delta(&path, &cursor, 2);
    assert_eq!(second.text, "A");
    assert_eq!(second.start_cursor, 3);
    assert_eq!(second.end_cursor, 4);
}

#[test]
fn non_utf8_output_advances_exact_byte_cursor_without_replay() {
    let root = TempDirGuard::new("non_utf8_output");
    let path = root.path().join("stdout");
    let cursor = root.path().join("stdout_cursor");
    std::fs::write(&path, b"ok\xfftail").expect("write non-utf8 output");

    let first = read_output_delta(&path, &cursor, 3);
    assert_eq!(first.text, "ok\u{fffd}");
    assert_eq!(first.start_cursor, 0);
    assert_eq!(first.end_cursor, 3);
    assert_eq!(first.encoding, "utf-8-lossy");

    let second = read_output_delta(&path, &cursor, 8);
    assert_eq!(second.text, "tail");
    assert_eq!(second.start_cursor, 3);
    assert_eq!(second.end_cursor, 7);
    assert_eq!(second.encoding, "utf-8");
}

#[test]
fn cancellation_escalation_is_inert_without_durable_request() {
    let root = TempDirGuard::new("cancel_inert");
    assert_eq!(maybe_escalate_cancel(root.path(), 100), "not_requested");
}

#[test]
fn terminal_record_prevents_late_cancellation_escalation() {
    let root = TempDirGuard::new("terminal_cancel");
    std::fs::write(root.path().join("cancel_requested_at"), "10").expect("write cancel marker");
    std::fs::write(root.path().join("exit_code"), "143").expect("write terminal record");

    assert_eq!(maybe_escalate_cancel(root.path(), 100), "terminal_observed");
    assert!(!root.path().join("cancel_escalated_signal").exists());
}

#[test]
fn missing_process_requires_a_durable_grace_observation() {
    let root = TempDirGuard::new("missing_process");
    assert!(!process_loss_is_stable(
        root.path(),
        ProcessIdentityState::Missing,
        100,
        5,
    ));
    assert!(!process_loss_is_stable(
        root.path(),
        ProcessIdentityState::Missing,
        104,
        5,
    ));
    assert!(process_loss_is_stable(
        root.path(),
        ProcessIdentityState::Missing,
        105,
        5,
    ));
    assert!(process_loss_is_stable(
        root.path(),
        ProcessIdentityState::Missing,
        105,
        5,
    ));
}

#[test]
fn identity_mismatch_waits_for_a_terminal_record_grace_window() {
    let root = TempDirGuard::new("identity_mismatch_grace");
    assert!(!process_loss_is_stable(
        root.path(),
        ProcessIdentityState::IdentityMismatch,
        100,
        5,
    ));
    assert!(!process_loss_is_stable(
        root.path(),
        ProcessIdentityState::IdentityMismatch,
        104,
        5,
    ));
    assert!(process_loss_is_stable(
        root.path(),
        ProcessIdentityState::IdentityMismatch,
        105,
        5,
    ));
}

#[cfg(unix)]
#[test]
fn process_identity_accepts_a_persisted_non_shell_command_marker() {
    use std::process::Command;

    let root = TempDirGuard::new("durable_runner_identity_marker");
    let executable = std::path::Path::new("/bin/sleep");
    let mut child = Command::new(executable)
        .arg("30")
        .spawn()
        .expect("spawn non-shell process");
    std::fs::write(
        root.path().join("process_command_marker"),
        executable.display().to_string(),
    )
    .expect("write process marker");
    std::thread::sleep(std::time::Duration::from_millis(25));

    let identity = process_identity_state(root.path(), child.id());
    let _ = child.kill();
    let _ = child.wait();

    assert_eq!(identity, ProcessIdentityState::AliveVerified);
}

#[cfg(unix)]
#[test]
fn durable_cancellation_escalates_a_verified_process_group() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let root = TempDirGuard::new("cancel_escalation");
    let run_script = root.path().join("run.sh");
    std::fs::write(&run_script, "#!/usr/bin/env bash\ntrap '' TERM\nsleep 30\n")
        .expect("write script");
    let mut command = Command::new("bash");
    command.arg(&run_script).process_group(0);
    let mut child = command.spawn().expect("spawn process group");
    std::fs::write(root.path().join("pid"), child.id().to_string()).expect("write pid");
    std::fs::write(root.path().join("cancel_requested_at"), "10").expect("write cancel marker");
    std::fs::write(root.path().join("terminate_grace_seconds"), "1").expect("write grace");
    std::thread::sleep(std::time::Duration::from_millis(25));

    assert_eq!(maybe_escalate_cancel(root.path(), 12), "kill_sent");
    let _ = child.wait();
    assert_eq!(
        std::fs::read_to_string(root.path().join("cancel_escalated_signal"))
            .expect("escalation marker"),
        "KILL"
    );
}

#[cfg(unix)]
#[test]
fn restart_recovery_escalates_a_durable_pending_cancellation() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let workspace = TempDirGuard::new("cancel_restart_recovery");
    let job_dir = workspace
        .path()
        .join(".agent-runtime")
        .join("async_jobs")
        .join("job-1");
    std::fs::create_dir_all(&job_dir).expect("create async job directory");
    let run_script = job_dir.join("run.sh");
    std::fs::write(&run_script, "#!/usr/bin/env bash\ntrap '' TERM\nsleep 30\n")
        .expect("write script");
    let mut command = Command::new("bash");
    command.arg(&run_script).process_group(0);
    let mut child = command.spawn().expect("spawn process group");
    std::fs::write(job_dir.join("pid"), child.id().to_string()).expect("write pid");
    std::fs::write(job_dir.join("cancel_requested_at"), "10").expect("write cancel marker");
    std::fs::write(job_dir.join("terminate_grace_seconds"), "1").expect("write grace");
    std::thread::sleep(std::time::Duration::from_millis(25));

    assert_eq!(recover_pending_cancel_escalations(workspace.path(), 12), 1);
    let _ = child.wait();
    assert_eq!(
        std::fs::read_to_string(job_dir.join("cancel_escalated_signal"))
            .expect("escalation marker"),
        "KILL"
    );
}

#[cfg(unix)]
#[test]
fn wrapper_exit_keeps_its_live_process_group_supervisable() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let root = TempDirGuard::new("wrapper_exit_group");
    let run_script = root.path().join("run.sh");
    let child_pid_path = root.path().join("child_pid");
    std::fs::write(
        &run_script,
        format!(
            "#!/usr/bin/env bash\nsleep 30 >/dev/null 2>&1 &\nprintf '%s' \"$!\" > '{}'\n",
            child_pid_path.display()
        ),
    )
    .expect("write wrapper script");
    let mut command = Command::new("bash");
    command.arg(&run_script).process_group(0);
    let mut wrapper = command.spawn().expect("spawn wrapper group");
    let process_group_id = wrapper.id();
    wrapper.wait().expect("wrapper exits");
    let child_pid: u32 = std::fs::read_to_string(&child_pid_path)
        .expect("child pid")
        .parse()
        .expect("numeric child pid");

    assert_eq!(
        process_identity_state(root.path(), process_group_id),
        ProcessIdentityState::AliveVerified
    );
    assert!(terminate_verified_process_group(
        root.path(),
        process_group_id,
        "KILL"
    ));
    for _ in 0..50 {
        let child_alive = Command::new("kill")
            .args(["-0", &child_pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !child_alive {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("child process remained alive after process-group termination");
}

#[test]
fn identity_state_tokens_are_stable() {
    assert_eq!(
        ProcessIdentityState::AliveVerified.as_token(),
        "alive_verified"
    );
    assert_eq!(ProcessIdentityState::Missing.alive(), Some(false));
    assert_eq!(ProcessIdentityState::Unknown.alive(), None);
}

use super::*;

#[cfg(target_os = "linux")]
#[path = "local_process_scope_fixture_tests.rs"]
mod fixtures;

#[cfg(target_os = "linux")]
#[path = "local_process_owner_fixture_bootstrap_tests.rs"]
mod fixture_bootstrap;

#[cfg(target_os = "linux")]
pub(crate) const FIXTURE_ENV: &str = "APP_PROCESS_SCOPE_TEST_CASE";

#[cfg(target_os = "linux")]
pub(crate) const CONTROL_ENV: &str = "APP_OWNER_FIXTURE_CONTROL_EXPECTED";

#[cfg(target_os = "linux")]
pub(crate) fn fixture_command(case: &str, foreign_group: Option<u32>) -> Command {
    let module = module_path!().split_once("::").unwrap().1;
    let fixture = format!("{module}::owner_scope_isolated_fixture");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &fixture,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(FIXTURE_ENV, case)
        .env_remove(CONTROL_ENV);
    if let Some(group) = foreign_group {
        command.env("APP_PROCESS_SCOPE_TEST_FOREIGN_GROUP", group.to_string());
    }
    command
}

#[cfg(target_os = "linux")]
pub(crate) fn run_isolated(case: &str, foreign_group: Option<u32>) {
    let output = fixture_command(case, foreign_group).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("{stdout}");
    assert!(
        output.status.success(),
        "isolated scope case {case}: {stdout}\n{stderr}"
    );
    assert!(stdout.contains("running 1 test"));
    assert!(stdout.contains("1 passed; 0 failed"));
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Parent tests execute this exact fixture in an isolated owner process"]
fn owner_scope_isolated_fixture() {
    let case = std::env::var(FIXTURE_ENV).expect("isolated fixture case required");
    // This is the fresh disposable helper, not the parent test runner. Match
    // the real entry's closed descriptor boundary before private fixture input.
    fixture_bootstrap::seal();
    if !matches!(case.as_str(), "privacy_probe" | "privacy_scope_unprotected") {
        // This exact fixture is a fresh one-job helper. Enter before its
        // private test key/control inputs; never change the parent runner.
        unsafe { super::super::owner_privacy::OwnerProcessPrivacy::protect() }.unwrap();
    }
    fixtures::run(&case);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_kernel_completion_preserves_the_original_root_status() {
    run_isolated("direct", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_keeps_an_orphan_until_the_kernel_scope_is_empty() {
    run_isolated("orphan", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_cancels_a_nested_new_session_after_the_root_exits() {
    run_isolated("nested", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_cancellation_catches_descendants_adopted_after_the_first_signal() {
    run_isolated("adoption_after_cancel", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_unavailable_completion_probe_does_not_block_owned_cancel() {
    run_isolated("scope_recovery_cancel", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_unavailable_completion_probe_does_not_block_owned_escalation() {
    run_isolated("scope_recovery_escalation", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_observed_ownership_loss_never_authorizes_a_signal() {
    run_isolated("scope_recovery_lost", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_unavailable_probe_cannot_mask_a_reaped_reference() {
    run_isolated("scope_recovery_reaped", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_repeated_cancel_keeps_the_original_deadline() {
    run_isolated("scope_recovery_deadline", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_drop_reaps_its_own_remaining_descendants() {
    run_isolated("drop", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_context_admission_never_reaps_a_preexisting_child() {
    run_isolated("busy", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_context_admission_refuses_automatic_child_reaping() {
    run_isolated("sigchld", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_context_admission_refuses_an_already_reserved_process() {
    run_isolated("reserved", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_failed_spawn_cannot_be_replayed_in_the_same_owner() {
    run_isolated("spawn_failed", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_does_not_invent_a_deadline_for_uncancelled_work() {
    run_isolated("no_deadline", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_child_joining_a_foreign_group_does_not_authorize_that_group() {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    let mut foreign = Command::new("/bin/sh")
        .args(["-c", "read -r value"])
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let input = foreign.stdin.take().unwrap();
    run_isolated("foreign", Some(foreign.id()));
    assert_eq!(foreign.try_wait().unwrap(), None);
    drop(input);
    assert_eq!(foreign.wait().unwrap().code(), Some(1));
}

#[cfg(not(target_os = "linux"))]
#[test]
fn owner_scope_unsupported_platform_cannot_start_a_payload() {
    let mut command = Command::new("/usr/bin/true");
    assert!(matches!(
        unsafe { OwnedProcessScope::spawn(&mut command, Duration::ZERO) },
        Err(ScopeFailure::Unsupported)
    ));
    let mut unsupported = OwnedProcessScope {};
    assert!(unsupported.take_stdin().is_none());
    assert_eq!(unsupported.poll(), Err(ScopeFailure::Unsupported));
    assert_eq!(unsupported.cancel(), Err(ScopeFailure::Unsupported));
}

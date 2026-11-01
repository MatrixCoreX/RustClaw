use std::process::Stdio;

use super::super::instance::{observe_process, ProcessRunState};
use super::*;

fn pipe_group(script: &str) -> OwnedChildGroup {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    OwnedChildGroup::spawn(&mut command).unwrap()
}

fn wait_exit(group: &mut OwnedChildGroup) -> ChildExit {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(exit) = group.poll_exit().unwrap() {
            return exit;
        }
        assert!(Instant::now() < deadline, "owned child did not exit");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn wait_finish(group: &mut OwnedChildGroup) -> ChildExit {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(exit) = group.try_finish().unwrap() {
            return exit;
        }
        assert!(
            Instant::now() < deadline,
            "owned process group did not finish"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn owned_group_spawn_proves_parent_and_original_group_without_a_marker() {
    let mut group = pipe_group("read -r value");
    let input = group.take_stdin().unwrap();
    let observed = observe_process(group.id()).unwrap();
    assert_eq!(observed.parent_pid, std::process::id());
    assert_eq!(observed.process_group, group.id());
    assert_eq!(group.poll_exit(), Ok(None));
    drop(input);
    assert_eq!(wait_finish(&mut group).code, Some(1));
}

#[test]
fn owned_group_exit_observation_does_not_reap_the_kernel_reference() {
    let mut group = pipe_group("exit 7");
    let exit = wait_exit(&mut group);
    assert_eq!(
        exit,
        ChildExit {
            code: Some(7),
            signal: None
        }
    );
    for _ in 0..5 {
        assert_eq!(group.poll_exit(), Ok(Some(exit)));
    }
    #[cfg(target_os = "linux")]
    assert_eq!(
        observe_process(group.id()).unwrap().state,
        ProcessRunState::Zombie
    );
    assert_eq!(group.try_finish(), Ok(Some(exit)));
    assert_eq!(group.poll_exit(), Err(GroupFailure::AlreadyReaped));
    assert_eq!(
        group.signal(OwnedSignal::Kill),
        Err(GroupFailure::AlreadyReaped)
    );
}

#[test]
fn owned_group_rejects_a_reaped_reference_without_sending_a_signal() {
    let mut group = pipe_group("exit 3");
    wait_exit(&mut group);
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(group.id() as i32, &mut status, 0) },
        group.id() as i32
    );
    assert_eq!(group.poll_exit(), Err(GroupFailure::OwnershipLost));
    assert_eq!(
        group.signal(OwnedSignal::Kill),
        Err(GroupFailure::OwnershipLost)
    );
}

#[test]
fn owned_group_term_is_observed_before_terminal_reaping() {
    let mut group = pipe_group("read -r value");
    let _input = group.take_stdin().unwrap();
    group.signal(OwnedSignal::Terminate).unwrap();
    assert_eq!(wait_exit(&mut group).signal, Some(libc::SIGTERM));
    assert_eq!(wait_finish(&mut group).signal, Some(libc::SIGTERM));
}

#[test]
fn owned_group_kill_does_not_target_another_live_group() {
    let mut first = pipe_group("read -r value");
    let mut second = pipe_group("read -r value");
    let _first_input = first.take_stdin().unwrap();
    let second_input = second.take_stdin().unwrap();
    first.signal(OwnedSignal::Kill).unwrap();
    assert_eq!(wait_finish(&mut first).signal, Some(libc::SIGKILL));
    assert_eq!(second.poll_exit(), Ok(None));
    drop(second_input);
    assert_eq!(wait_finish(&mut second).code, Some(1));
}

#[test]
fn owned_group_child_joining_a_foreign_group_never_authorizes_that_group() {
    use std::os::unix::process::CommandExt;

    let mut foreign = pipe_group("read -r value");
    let foreign_input = foreign.take_stdin().unwrap();
    let foreign_group = foreign.id() as i32;
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "read -r value"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            if libc::setpgid(0, foreign_group) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let mut owned = OwnedChildGroup::spawn(&mut command).unwrap();
    let _owned_input = owned.take_stdin().unwrap();
    assert_eq!(
        observe_process(owned.id()).unwrap().process_group,
        foreign.id()
    );
    owned.signal(OwnedSignal::Kill).unwrap();
    assert_eq!(wait_finish(&mut owned).signal, Some(libc::SIGKILL));
    assert_eq!(foreign.poll_exit(), Ok(None));
    drop(foreign_input);
    assert_eq!(wait_finish(&mut foreign).code, Some(1));
}

#[test]
fn owned_group_keeps_a_live_orphan_after_its_leader_exits() {
    let root = GroupFixture::new();
    let child_record = root.0.join("child");
    let script = format!(
        "exec 3<&0\n/bin/sh -c 'read -r value' <&3 >/dev/null 2>&1 &\nprintf '%s' \"$!\" > '{}'",
        child_record.display()
    );
    let mut group = pipe_group(&script);
    let _input = group.take_stdin().unwrap();
    assert_eq!(wait_exit(&mut group).code, Some(0));
    let child: u32 = std::fs::read_to_string(child_record)
        .unwrap()
        .parse()
        .unwrap();
    let members = group_members(group.id()).unwrap();
    assert!(
        members
            .iter()
            .any(|member| member.birth.pid == child && member.state == ProcessRunState::Active),
        "child={child} members={members:?}"
    );
    assert_eq!(group.try_finish(), Ok(None));
    group.signal(OwnedSignal::Kill).unwrap();
    assert_eq!(wait_finish(&mut group).code, Some(0));
}

#[test]
fn owned_group_probe_rejects_special_targets_without_a_host_scan() {
    for group in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
        assert_eq!(
            group_members(group),
            Err(ProcessProbeFailure::InvalidTarget)
        );
    }
}

#[test]
fn owned_group_signal_mapping_is_closed_and_machine_typed() {
    assert_eq!(OwnedSignal::Interrupt.raw(), libc::SIGINT);
    assert_eq!(OwnedSignal::Terminate.raw(), libc::SIGTERM);
    assert_eq!(OwnedSignal::Kill.raw(), libc::SIGKILL);
}

#[test]
fn owned_group_missing_program_returns_without_an_owned_reference() {
    let root = GroupFixture::new();
    let mut command = Command::new(root.0.join("missing"));
    assert!(matches!(
        OwnedChildGroup::spawn(&mut command),
        Err(GroupFailure::SpawnFailed)
    ));
}

#[test]
fn owned_group_exit_status_is_stable_across_darwin_high_bit_observations() {
    let exit = ChildExit {
        code: Some(7),
        signal: None,
    };
    for raw in [7, 0x01000007, -249] {
        assert_eq!(decode_exit(libc::CLD_EXITED, raw), Ok(Some(exit)));
    }
    for kind in [libc::CLD_KILLED, libc::CLD_DUMPED] {
        assert_eq!(
            decode_exit(kind, libc::SIGKILL),
            Ok(Some(ChildExit {
                code: None,
                signal: Some(libc::SIGKILL)
            }))
        );
        assert_eq!(decode_exit(kind, 0), Err(GroupFailure::ProbeUnavailable));
    }
    assert_eq!(
        decode_exit(libc::CLD_STOPPED, libc::SIGSTOP),
        Err(GroupFailure::ProbeUnavailable)
    );
}

struct GroupFixture(std::path::PathBuf);

impl GroupFixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-process-group-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for GroupFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

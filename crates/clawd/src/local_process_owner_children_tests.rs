use super::*;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

fn held_child() -> (Child, std::process::ChildStdin) {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "read -r _ || :; exit 7"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    (child, input)
}

fn wait(observer: &HostOwnerChild) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = observer.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "owned_child_not_reaped");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn retained_child_survives_host_drop_and_is_reaped_without_killing() {
    let reaper = OwnerChildReaper::new().unwrap();
    let (child, mut release) = held_child();
    let observer = reaper.retain(child).unwrap();
    drop(reaper);
    assert!(observer.try_wait().unwrap().is_none());
    release.write_all(b"finish\n").unwrap();
    drop(release);
    assert_eq!(wait(&observer).code(), Some(7));
    assert_eq!(observer.try_wait().unwrap().unwrap().code(), Some(7));
}

#[test]
fn concurrent_owned_children_keep_independent_observations() {
    let reaper = OwnerChildReaper::new().unwrap();
    let (first, first_input) = held_child();
    let (second, second_input) = held_child();
    let first = reaper.retain(first).unwrap();
    let second = reaper.clone().retain(second).unwrap();
    drop(first_input);
    assert_eq!(wait(&first).code(), Some(7));
    assert!(second.try_wait().unwrap().is_none());
    drop(second_input);
    assert_eq!(wait(&second).code(), Some(7));
}

#[cfg(target_os = "linux")]
#[test]
fn dropping_every_observer_and_host_still_reaps_the_owned_child() {
    let reaper = OwnerChildReaper::new().unwrap();
    let (child, release) = held_child();
    let pid = child.id();
    let observer = reaper.retain(child).unwrap();
    assert_eq!(observer.id(), pid);
    drop(observer);
    drop(reaper);
    drop(release);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            assert_eq!(
                io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
            break;
        }
        assert!(Instant::now() < end, "dropped_observers_lost_child_custody");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn closed_registration_returns_the_exact_child_before_launch_transfer() {
    let (sender, receiver) = mpsc::channel();
    drop(receiver);
    let reaper = OwnerChildReaper(sender);
    let (child, release) = held_child();
    let pid = child.id();
    let mut returned = match reaper.retain(child) {
        Err(child) => child,
        Ok(_) => panic!("closed_child_registration_accepted"),
    };
    assert_eq!(returned.id(), pid);
    assert!(returned.try_wait().unwrap().is_none());
    drop(release);
    assert_eq!(returned.wait().unwrap().code(), Some(7));
}

#[cfg(target_os = "linux")]
#[test]
fn lost_kernel_child_reference_is_unknown_and_unrelated_child_is_not_reaped() {
    let reaper = OwnerChildReaper::new().unwrap();
    let (child, release) = held_child();
    let pid = child.id();
    drop(release);
    assert_eq!(
        unsafe { libc::waitpid(pid as i32, std::ptr::null_mut(), 0) },
        pid as i32
    );
    let observed = reaper.retain(child).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        match observed.try_wait() {
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
                break;
            }
            Ok(None) => assert!(Instant::now() < end, "lost_child_was_not_unknown"),
            Ok(Some(_)) => panic!("lost_child_was_fabricated_as_reaped"),
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let (mut unrelated, release) = held_child();
    let pid = unrelated.id();
    drop(release);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        assert_eq!(
            unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid,
                    info.as_mut_ptr(),
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            },
            0
        );
        if unsafe { info.assume_init().si_pid() } == pid as i32 {
            break;
        }
        assert!(Instant::now() < end, "unrelated_fixture_not_exited");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(unrelated.wait().unwrap().code(), Some(7));
    assert!(observed.try_wait().is_err());
}

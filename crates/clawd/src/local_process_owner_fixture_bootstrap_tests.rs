use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;

use super::CONTROL_ENV;
const CONTROL_FD: i32 = 198;

fn inheritable_descriptors() -> Vec<i32> {
    let mut descriptors: Vec<_> = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter_map(|name| name.to_str().and_then(|value| value.parse::<i32>().ok()))
        .filter(|fd| *fd > 2)
        .filter(|fd| {
            let flags = unsafe { libc::fcntl(*fd, libc::F_GETFD) };
            flags >= 0 && flags & libc::FD_CLOEXEC == 0
        })
        .collect();
    descriptors.sort_unstable();
    descriptors
}

pub(super) fn seal() {
    let before = inheritable_descriptors();
    let control = std::env::var(CONTROL_ENV).as_deref() == Ok("1");
    // Only this freshly exec'd one-job test process owns this table. Never
    // repair a production scope's caller descriptors or the Cargo/daemon table.
    if control {
        let flags = unsafe { libc::fcntl(CONTROL_FD, libc::F_GETFD) };
        assert!(flags >= 0, "fixture control descriptor missing");
        assert_eq!(
            unsafe { libc::fcntl(CONTROL_FD, libc::F_SETFD, flags | libc::FD_CLOEXEC) },
            0
        );
        assert_eq!(
            unsafe { libc::syscall(libc::SYS_close_range, 3_u32, (CONTROL_FD - 1) as u32, 0_u32) },
            0
        );
        assert_eq!(
            unsafe {
                libc::syscall(
                    libc::SYS_close_range,
                    (CONTROL_FD + 1) as u32,
                    u32::MAX,
                    0_u32,
                )
            },
            0
        );
    } else {
        assert_eq!(
            unsafe { libc::syscall(libc::SYS_close_range, 3_u32, u32::MAX, 0_u32) },
            0
        );
    }
    let after = inheritable_descriptors();
    assert!(after.is_empty());
    eprintln!(
        "{}",
        serde_json::json!({
            "owner_fixture_fd_bootstrap": true,
            "ambient_non_cloexec": before,
            "control_preserved": control,
            "remaining_non_cloexec": after,
        })
    );
}

#[test]
fn owner_fixture_seals_child_inheritance_without_changing_parent_descriptors() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("marker");
    std::fs::write(&marker, b"owned-parent-marker").unwrap();
    let original = std::fs::File::open(&marker).unwrap();
    let flags = unsafe { libc::fcntl(original.as_raw_fd(), libc::F_GETFD) };
    let source = original.try_clone().unwrap();
    let mut command = super::fixture_command("direct", None);
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(source.as_raw_fd(), 197) < 0 || libc::fcntl(197, libc::F_SETFD, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|value| value["owner_fixture_fd_bootstrap"] == true)
        .expect("actual fixture descriptor report");
    assert!(report["ambient_non_cloexec"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!(197)));
    assert_eq!(report["remaining_non_cloexec"], serde_json::json!([]));
    assert_eq!(report["control_preserved"], false);
    assert_eq!(
        unsafe { libc::fcntl(original.as_raw_fd(), libc::F_GETFD) },
        flags
    );
    assert_eq!(
        original.metadata().unwrap().len(),
        b"owned-parent-marker".len() as u64
    );
    assert_eq!(std::fs::read(marker).unwrap(), b"owned-parent-marker");
    println!("{report}");
}

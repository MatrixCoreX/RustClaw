use std::fs::{self, File};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use super::InstallReceiptStore;

struct Fixture {
    _root: tempfile::TempDir,
    store: InstallReceiptStore,
    version: PathBuf,
    ready: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = InstallReceiptStore::new(root.path().join("packages"));
        let version = store
            .version_dir("lease_fixture", "1.0.0", &"a".repeat(64))
            .unwrap();
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join("artifact"), b"owned lease fixture").unwrap();
        Self {
            ready: root.path().join("ready"),
            _root: root,
            store,
            version,
        }
    }
}

struct LeaseChild {
    child: Child,
    input: Option<ChildStdin>,
}

impl LeaseChild {
    fn start(lease: File, ready: &Path) -> Self {
        let descriptor = unsafe { libc::fcntl(lease.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 4) };
        assert!(descriptor >= 4);
        let pinned = unsafe { File::from_raw_fd(descriptor) };
        drop(lease);
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "printf ready > \"$1\"; read -r _ || :", "lease-child"])
            .arg(ready)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Only this owned fixture inherits fd3; the real owner must use CLOEXEC
        // to keep its lease descriptor out of every payload.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(pinned.as_raw_fd(), 3) != 3 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        drop(command);
        let input = child.stdin.take();
        let mut owned = Self { child, input };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            assert!(owned.child.try_wait().unwrap().is_none());
            assert!(Instant::now() < deadline, "owned lease child not ready");
            std::thread::sleep(Duration::from_millis(5));
        }
        owned
    }

    fn finish(&mut self) -> ExitStatus {
        drop(self.input.take());
        self.child.wait().unwrap()
    }
}

impl Drop for LeaseChild {
    fn drop(&mut self) {
        drop(self.input.take());
        let _ = self.child.wait();
    }
}

#[test]
fn process_owner_keeps_version_leased_after_dispatch_guard_drops() {
    // An owned SDK version-directory/GC fixture, not package admission proof.
    let fixture = Fixture::new();
    let lease = fixture
        .store
        .acquire_version_lease("lease_fixture", &fixture.version)
        .unwrap();
    let mut child = LeaseChild::start(lease.retain_process_handle().unwrap(), &fixture.ready);
    assert!(!fixture
        .store
        .remove_installed_versions("lease_fixture")
        .unwrap());
    drop(lease);
    assert!(child.child.try_wait().unwrap().is_none());
    assert!(
        fixture.version.is_dir(),
        "dispatch guard released a live process owner's package"
    );
    assert!(!fixture
        .store
        .remove_installed_versions("lease_fixture")
        .unwrap());
    assert!(child.finish().success());
    assert!(fixture
        .store
        .remove_installed_versions("lease_fixture")
        .unwrap());
    assert!(!fixture.version.exists());
}

#[test]
fn non_transferred_dispatch_lease_still_collects_after_its_guard_drops() {
    let fixture = Fixture::new();
    let lease = fixture
        .store
        .acquire_version_lease("lease_fixture", &fixture.version)
        .unwrap();
    assert!(!fixture
        .store
        .remove_installed_versions("lease_fixture")
        .unwrap());
    drop(lease);
    assert!(!fixture.version.exists());
}

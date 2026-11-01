use super::*;

#[cfg(target_os = "linux")]
#[path = "local_process_owner_privacy_fixture_tests.rs"]
mod fixtures;

#[cfg(target_os = "linux")]
use std::fs::{self, File};
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::{DirBuilderExt, FileExt};
#[cfg(target_os = "linux")]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(target_os = "linux")]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::Child;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
pub(super) const MARKER: &[u8] = b"non-secret-owner-memory-v1";

#[cfg(target_os = "linux")]
struct Probe {
    root: PathBuf,
    child: Child,
    peer: UnixStream,
    pid: u32,
    fd: i32,
    address: usize,
}

#[cfg(target_os = "linux")]
impl Probe {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("pv-{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        fs::write(root.join("marker"), MARKER).unwrap();
        let listener = UnixListener::bind(root.join("s")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut command = super::super::owner_scope::tests::fixture_command("privacy_probe", None);
        command.env_clear().env(
            super::super::owner_scope::tests::FIXTURE_ENV,
            "privacy_probe",
        );
        for name in ["PATH", "TMPDIR"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let child = command
            .env("APP_PRIVATE_PROCESS_FIXTURE_ROOT", &root)
            .env(
                "APP_PRIVATE_PROCESS_FIXTURE_MARKER",
                "non-secret-environment-marker",
            )
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut probe = Self {
            root,
            pid: child.id(),
            child,
            peer: UnixStream::pair().unwrap().0,
            fd: -1,
            address: 0,
        };
        loop {
            match listener.accept() {
                Ok((peer, _)) => {
                    probe.peer = peer;
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "private fixture did not connect");
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("{error}"),
            }
        }
        probe
            .peer
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        probe
            .peer
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = [0; 12];
        probe.peer.read_exact(&mut bytes).unwrap();
        probe.fd = i32::from_be_bytes(bytes[..4].try_into().unwrap());
        probe.address = u64::from_be_bytes(bytes[4..].try_into().unwrap())
            .try_into()
            .unwrap();
        assert!(probe.fd > 2 && probe.address > 0);
        probe
    }

    fn step(&mut self, byte: u8) {
        self.peer.write_all(&[byte]).unwrap();
        let mut reply = [0];
        self.peer.read_exact(&mut reply).unwrap();
        assert_eq!(reply, [byte]);
    }

    fn memory_read(&self) -> std::io::Result<Vec<u8>> {
        let file = File::open(format!("/proc/{}/mem", self.pid))?;
        let mut bytes = vec![0; MARKER.len()];
        file.read_exact_at(&mut bytes, self.address as u64)?;
        Ok(bytes)
    }

    fn vm_read(&self) -> std::io::Result<Vec<u8>> {
        let mut bytes = vec![0; MARKER.len()];
        let local = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let remote = libc::iovec {
            iov_base: self.address as *mut libc::c_void,
            iov_len: bytes.len(),
        };
        let count = unsafe { libc::process_vm_readv(self.pid as i32, &local, 1, &remote, 1, 0) };
        if count < 0 {
            return Err(std::io::Error::last_os_error());
        }
        assert_eq!(count as usize, bytes.len());
        Ok(bytes)
    }

    fn assert_visible(&self) {
        assert_eq!(self.memory_read().unwrap(), MARKER);
        assert_eq!(self.vm_read().unwrap(), MARKER);
        assert_eq!(
            fs::read(format!("/proc/{}/fd/{}", self.pid, self.fd)).unwrap(),
            MARKER
        );
        assert!(fs::read_link(format!("/proc/{}/fd/{}", self.pid, self.fd)).is_ok());
        assert_eq!(
            fs::read(format!(
                "/proc/{}/root{}/marker",
                self.pid,
                self.root.display()
            ))
            .unwrap(),
            MARKER
        );
        let environment = fs::read(format!("/proc/{}/environ", self.pid)).unwrap();
        assert!(environment
            .split(|byte| *byte == 0)
            .any(|entry| entry
                == b"APP_PRIVATE_PROCESS_FIXTURE_MARKER=non-secret-environment-marker"));
    }

    fn assert_hidden(&self) {
        assert_denied(self.memory_read());
        assert_denied(self.vm_read());
        assert_denied(fs::read(format!("/proc/{}/fd/{}", self.pid, self.fd)));
        assert_denied(fs::read_link(format!("/proc/{}/fd/{}", self.pid, self.fd)));
        assert_denied(fs::read(format!(
            "/proc/{}/root{}/marker",
            self.pid,
            self.root.display()
        )));
        assert_denied(fs::read(format!("/proc/{}/environ", self.pid)));
        assert_eq!(fs::read(self.root.join("marker")).unwrap(), MARKER);
    }

    fn finish(mut self) {
        self.step(b'X');
        let status = self.child.wait().unwrap();
        assert!(status.success());
    }
}

#[cfg(target_os = "linux")]
impl Drop for Probe {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[cfg(target_os = "linux")]
fn assert_denied<T>(result: std::io::Result<T>) {
    let error = result
        .err()
        .expect("private process observation unexpectedly succeeded");
    assert!(
        matches!(error.raw_os_error(), Some(libc::EACCES | libc::EPERM)),
        "{error}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn private_owner_process_blocks_real_parent_proc_fd_memory_and_environment_reads() {
    let mut probe = Probe::new();
    probe.assert_visible();
    let birth = super::super::instance::observe_process(probe.pid)
        .unwrap()
        .birth;
    probe.step(b'P');
    probe.assert_hidden();
    assert_eq!(
        super::super::instance::observe_process(probe.pid)
            .unwrap()
            .birth,
        birth
    );
    println!("owner_privacy_parent_control=visible protected_proc_fd_mem_vm_environ=denied birth_identity=unchanged");
    probe.finish();
}

#[cfg(target_os = "linux")]
#[test]
fn private_owner_guard_detects_disabled_protection_and_can_protect_before_loading_new_data() {
    let mut probe = Probe::new();
    probe.step(b'P');
    probe.assert_hidden();
    probe.step(b'R');
    probe.assert_visible();
    probe.step(b'P');
    probe.assert_hidden();
    probe.finish();
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_admission_requires_prior_privacy_and_rechecks_before_observation_or_cancel() {
    super::super::owner_scope::tests::run_isolated("privacy_scope_unprotected", None);
    super::super::owner_scope::tests::run_isolated("privacy_scope_lost", None);
}

#[cfg(target_os = "linux")]
#[test]
fn private_owner_guard_cannot_be_reused_by_a_forked_process() {
    super::super::owner_scope::tests::run_isolated("privacy_fork", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_rejects_ambient_descriptor_before_starting_a_payload() {
    super::super::owner_scope::tests::run_isolated("privacy_descriptor", None);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_scope_rejects_ambient_descriptor_above_the_current_fd_limit() {
    super::super::owner_scope::tests::run_isolated("privacy_high_descriptor", None);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn unsupported_owner_privacy_is_not_silently_treated_as_protected() {
    assert!(matches!(
        unsafe { OwnerProcessPrivacy::protect() },
        Err(ScopeFailure::Unsupported)
    ));
}

#[cfg(target_os = "linux")]
pub(crate) fn run_case(case: &str) {
    fixtures::run(case);
}

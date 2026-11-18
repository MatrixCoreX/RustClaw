use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::super::super::owner_scope::OwnedProcessScope;
use super::{OwnerProcessPrivacy, ScopeFailure, MARKER};

pub(super) fn run(case: &str) {
    match case {
        "privacy_probe" => probe(),
        "privacy_scope_lost" => scope(),
        "privacy_scope_unprotected" => unprotected(),
        "privacy_fork" => forked_guard(),
        "privacy_descriptor" => descriptor(false),
        "privacy_high_descriptor" => descriptor(true),
        _ => panic!("unknown privacy fixture"),
    }
}

struct DescriptorFixture {
    root: std::path::PathBuf,
    original_limit: libc::rlimit,
}

impl Drop for DescriptorFixture {
    fn drop(&mut self) {
        assert_eq!(
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &self.original_limit) },
            0
        );
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn descriptor(high: bool) {
    let root = std::env::temp_dir().join(format!("df-{}", uuid::Uuid::new_v4().simple()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let mut original_limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut original_limit) },
        0
    );
    let fixture = DescriptorFixture {
        root,
        original_limit,
    };
    fs::write(fixture.root.join("marker"), MARKER).unwrap();
    let original = File::open(fixture.root.join("marker")).unwrap();
    let minimum = if high { 1024 } else { 3 };
    let raw = unsafe { libc::fcntl(original.as_raw_fd(), libc::F_DUPFD, minimum) };
    assert!(raw >= minimum);
    let ambient = unsafe { File::from_raw_fd(raw) };
    assert_eq!(
        unsafe { libc::fcntl(raw, libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
    if high {
        let mut limit = fixture.original_limit;
        limit.rlim_cur = 128;
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
        assert!(raw as u64 >= limit.rlim_cur);
    }
    let mut payload = Command::new("/bin/sh");
    payload
        .args([
            "-c",
            "printf effect > started; if cat \"$1\" > leaked 2>/dev/null; then exit 9; fi; exit 0",
            "fixture",
        ])
        .arg(format!("/proc/self/fd/{raw}"))
        .current_dir(&fixture.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match unsafe { OwnedProcessScope::spawn(&mut payload, Duration::ZERO) } {
        Err(ScopeFailure::InvalidDescriptor) => {}
        Err(error) => panic!("unexpected descriptor refusal: {error:?}"),
        Ok(mut scope) => {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !scope.poll().unwrap().complete {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(fs::read(fixture.root.join("leaked")).unwrap(), MARKER);
            panic!("non_cloexec_test_marker_was_inherited high={high}");
        }
    }
    assert!(!fixture.root.join("started").exists());
    assert_eq!(unsafe { libc::fcntl(raw, libc::F_GETFD) }, 0);
    assert_eq!(ambient.metadata().unwrap().len(), MARKER.len() as u64);
    let mut enabled = -1_i32;
    assert_eq!(
        unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut enabled, 0, 0, 0) },
        0
    );
    assert_eq!(enabled, 0);
    // The rejected handle stays usable; only its explicit inheritance policy changes.
    assert_eq!(
        unsafe { libc::fcntl(ambient.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
        0
    );
    let mut scope = unsafe { OwnedProcessScope::spawn(&mut payload, Duration::ZERO) }.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let terminal = loop {
        let snapshot = scope.poll().unwrap();
        if snapshot.complete {
            break snapshot;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(terminal.root_exit.unwrap().code, Some(0));
    assert!(fixture.root.join("started").exists());
    assert!(fs::read(fixture.root.join("leaked")).unwrap().is_empty());
    assert_eq!(fs::read(fixture.root.join("marker")).unwrap(), MARKER);
}

fn unprotected() {
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 1 as libc::c_ulong, 0, 0, 0) },
        0
    );
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 0"]);
    assert!(matches!(
        unsafe { OwnedProcessScope::spawn(&mut command, Duration::ZERO) },
        Err(ScopeFailure::OwnershipLost)
    ));
    assert_eq!(unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) }, 1);
    let mut status = 0;
    assert_eq!(unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
    let mut enabled = -1_i32;
    assert_eq!(
        unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut enabled, 0, 0, 0) },
        0
    );
    assert_eq!(enabled, 0);
}

fn forked_guard() {
    let privacy = OwnerProcessPrivacy::admit_current().unwrap();
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0);
    if pid == 0 {
        // No allocation, locking, destructors or test harness after fork.
        // The PID mismatch returns before any other observation or side effect.
        let status = if privacy.validate() == Err(ScopeFailure::OwnershipLost) {
            0
        } else {
            7
        };
        unsafe { libc::_exit(status) };
    }
    let mut status = 0;
    assert_eq!(unsafe { libc::waitpid(pid, &mut status, 0) }, pid);
    assert!(libc::WIFEXITED(status));
    assert_eq!(libc::WEXITSTATUS(status), 0);
    privacy.validate().unwrap();
}

fn probe() {
    let allowed = [
        super::super::super::owner_scope::tests::FIXTURE_ENV,
        "PATH",
        "TMPDIR",
        "APP_PRIVATE_PROCESS_FIXTURE_ROOT",
        "APP_PRIVATE_PROCESS_FIXTURE_MARKER",
    ];
    assert!(std::env::vars_os().all(|(name, _)| allowed
        .iter()
        .any(|allowed| name == std::ffi::OsStr::new(allowed))));
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 1 as libc::c_ulong, 0, 0, 0) },
        0
    );
    let root =
        std::path::PathBuf::from(std::env::var_os("APP_PRIVATE_PROCESS_FIXTURE_ROOT").unwrap());
    let file = File::open(root.join("marker")).unwrap();
    let marker = MARKER.to_vec().into_boxed_slice();
    let mut peer = UnixStream::connect(root.join("s")).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    peer.set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    peer.write_all(&file.as_raw_fd().to_be_bytes()).unwrap();
    peer.write_all(&(marker.as_ptr() as usize as u64).to_be_bytes())
        .unwrap();
    let mut privacy = None;
    loop {
        let mut byte = [0];
        peer.read_exact(&mut byte).unwrap();
        match byte[0] {
            b'P' => {
                privacy = Some(unsafe { OwnerProcessPrivacy::protect() }.unwrap());
                privacy.as_ref().unwrap().validate().unwrap();
            }
            b'R' => {
                assert_eq!(
                    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 1 as libc::c_ulong, 0, 0, 0) },
                    0
                );
                assert_eq!(
                    privacy.as_ref().unwrap().validate(),
                    Err(ScopeFailure::OwnershipLost)
                );
            }
            b'X' => {
                peer.write_all(&byte).unwrap();
                assert_eq!(&*marker, MARKER);
                assert_eq!(fs::read(root.join("marker")).unwrap(), MARKER);
                return;
            }
            _ => panic!("invalid fixture phase"),
        }
        peer.write_all(&byte).unwrap();
    }
}

fn scope() {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "read -r value"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut scope = unsafe { OwnedProcessScope::spawn(&mut command, Duration::ZERO) }.unwrap();
    let input = scope.take_stdin().unwrap();
    assert_eq!(unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) }, 0);
    assert!(!scope.poll().unwrap().complete);
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 1 as libc::c_ulong, 0, 0, 0) },
        0
    );
    assert_eq!(scope.poll(), Err(ScopeFailure::OwnershipLost));
    assert_eq!(scope.cancel(), Err(ScopeFailure::OwnershipLost));
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0 as libc::c_ulong, 0, 0, 0) },
        0
    );
    assert!(!scope.poll().unwrap().cancel_requested);
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !scope.poll().unwrap().complete {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 1 as libc::c_ulong, 0, 0, 0) },
        0
    );
    assert_eq!(scope.poll(), Err(ScopeFailure::OwnershipLost));
    assert_eq!(scope.cancel(), Err(ScopeFailure::OwnershipLost));
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0 as libc::c_ulong, 0, 0, 0) },
        0
    );
    assert!(scope.poll().unwrap().complete);
}

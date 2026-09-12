use super::*;
use std::{os::fd::AsRawFd, os::unix::net::UnixStream, process::Command};

#[test]
fn worker_protocol_rejects_duplicate_unknown_and_secret_export_fields() {
    for request in [
        r#"{"operation":"status","unexpected":true}"#,
        r#"{"operation":"lock","unexpected":true}"#,
        r#"{"operation":"initialize","password":"test-only-vault-password","password":"duplicate"}"#,
        r#"{"operation":"export_private_key"}"#,
    ] {
        assert!(
            serde_json::from_str::<protocol::Request>(request).is_err(),
            "{request}"
        );
    }
}

#[test]
fn bounded_transport_rejects_oversized_partial_and_closed_frames() {
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    transport::nonblocking(reader.as_raw_fd()).unwrap();
    transport::nonblocking(writer.as_raw_fd()).unwrap();
    transport::write(&mut writer, b"public response", Duration::from_secs(1)).unwrap();
    assert_eq!(
        &*transport::read(&mut reader, Duration::from_secs(1)).unwrap(),
        b"public response"
    );
    use std::io::Write;
    writer.write_all(&u32::MAX.to_be_bytes()).unwrap();
    assert_eq!(
        transport::read(&mut reader, Duration::from_millis(30)).unwrap_err(),
        "wallet_worker_protocol_invalid"
    );
    writer.write_all(&10u32.to_be_bytes()).unwrap();
    writer.write_all(b"abc").unwrap();
    assert_eq!(
        transport::read(&mut reader, Duration::from_millis(30)).unwrap_err(),
        "wallet_worker_timeout"
    );
    drop(writer);
    assert_eq!(
        transport::read(&mut reader, Duration::from_millis(30)).unwrap_err(),
        "wallet_worker_unavailable"
    );
}

#[test]
fn native_process_protections_and_mandatory_locked_memory() {
    if let Ok(mode) = std::env::var("DESKTOP_PROTECTION_TEST_CHILD") {
        if mode == "memory-denied" {
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &limit) }, 0);
            assert_eq!(
                crate::wallet::secure_memory::LockedKey::zeroed()
                    .err()
                    .unwrap(),
                "wallet_memory_protection_unavailable"
            );
        } else {
            // Inherit an actual Internet socket and prove it is closed, too.
            let fd = std::env::var("DESKTOP_PROTECTION_TEST_FD")
                .unwrap()
                .parse::<i32>()
                .unwrap();
            assert!(unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0);
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let other = barrier.clone();
            let thread = std::thread::spawn(move || {
                other.wait();
                assert_eq!(
                    unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) },
                    -1
                );
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::EPERM)
                );
            });
            sandbox::enter().unwrap();
            barrier.wait();
            thread.join().unwrap();
            assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
            let mut limit = libc::rlimit {
                rlim_cur: 1,
                rlim_max: 1,
            };
            assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) }, 0);
            assert_eq!((limit.rlim_cur, limit.rlim_max), (0, 0));
            for family in [
                libc::AF_INET,
                libc::AF_INET6,
                libc::AF_NETLINK,
                libc::AF_PACKET,
            ] {
                assert_eq!(unsafe { libc::socket(family, libc::SOCK_STREAM, 0) }, -1);
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::EPERM)
                );
            }
            let pair = UnixStream::pair().unwrap();
            drop(pair);
            for option in [libc::PR_SET_DUMPABLE, libc::PR_SET_PDEATHSIG] {
                assert_eq!(unsafe { libc::prctl(option, 1, 0, 0, 0) }, -1);
                assert_eq!(
                    std::io::Error::last_os_error().raw_os_error(),
                    Some(libc::EPERM)
                );
            }
            assert_eq!(unsafe { libc::ptrace(libc::PTRACE_TRACEME, 0, 0, 0) }, -1);
            let name = c"/this-executable-does-not-exist";
            assert_eq!(
                unsafe { libc::execve(name.as_ptr(), std::ptr::null(), std::ptr::null()) },
                -1
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EPERM)
            );
            assert!(Command::new("/bin/true").spawn().is_err());
            let key = crate::wallet::secure_memory::LockedKey::random().unwrap();
            let address = key.as_ptr() as usize;
            let maps = std::fs::read_to_string("/proc/self/smaps").unwrap();
            let mut current = false;
            let mut matched = false;
            for line in maps.lines() {
                if let Some(range) = line.split_whitespace().next().filter(|s| s.contains('-')) {
                    if let Some((start, end)) = range.split_once('-') {
                        if let (Ok(start), Ok(end)) = (
                            usize::from_str_radix(start, 16),
                            usize::from_str_radix(end, 16),
                        ) {
                            current = (start..end).contains(&address);
                            if current {
                                assert!(line.contains("rw-p"));
                            }
                        }
                    }
                }
                if current && line.starts_with("VmFlags:") {
                    let flags: Vec<_> = line.split_whitespace().collect();
                    assert!(flags.contains(&"lo") && flags.contains(&"dd"));
                    matched = true;
                }
            }
            assert!(matched);
            drop(key);
        }
        std::process::exit(0);
    }
    for mode in ["sandbox", "memory-denied"] {
        let socket = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        assert!(socket >= 3);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "wallet::worker::tests::native_process_protections_and_mandatory_locked_memory",
                "--nocapture",
            ])
            .env("DESKTOP_PROTECTION_TEST_CHILD", mode)
            .env("DESKTOP_PROTECTION_TEST_FD", socket.to_string())
            .output()
            .unwrap();
        unsafe {
            libc::close(socket);
        }
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

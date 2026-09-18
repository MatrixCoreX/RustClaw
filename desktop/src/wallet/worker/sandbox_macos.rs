//! Seatbelt is mandatory here, including on ad-hoc internal builds. It is a
//! deprecated OS API; unsupported future releases fail closed, never bypass it.
use crate::Result;
use std::{
    ffi::{c_char, c_int},
    mem::zeroed,
    ptr::{null, null_mut},
    time::Duration,
};

#[link(name = "sandbox")]
unsafe extern "C" {
    fn sandbox_init(profile: *const c_char, flags: u64, error: *mut *mut c_char) -> c_int;
    fn sandbox_free_error(error: *mut c_char);
}
fn protection_error(stage: &str) -> String {
    // Only static stage and errno, before any vault credentials are opened.
    eprintln!(
        "wallet_macos_protection stage={stage} errno={:?}",
        std::io::Error::last_os_error().raw_os_error()
    );
    "wallet_process_protection_unavailable".into()
}
pub fn enter() -> Result<()> {
    unsafe {
        close_inherited().map_err(|_| protection_error("close_inherited"))?;
        let parent = libc::getppid();
        if parent <= 1 {
            return Err(protection_error("parent"));
        }
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0
            || libc::ptrace(libc::PT_DENY_ATTACH, 0, null_mut(), 0) != 0
        {
            return Err(protection_error("core_or_ptrace"));
        }
        // Register before applying process-info restrictions, then check for the
        // parent-exit race. A dedicated kernel event kills even a busy KDF worker.
        let queue = libc::kqueue();
        if queue < 0 {
            return Err(protection_error("kqueue"));
        }
        let event = libc::kevent {
            ident: parent as usize,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: null_mut(),
        };
        if libc::kevent(queue, &event, 1, null_mut(), 0, null()) != 0 || libc::getppid() != parent {
            libc::close(queue);
            return Err(protection_error("parent_watch"));
        }
        // Keychain uses Mach IPC. No network socket or executable child is needed.
        // This is not a filesystem sandbox: selected backup paths remain allowed.
        let profile =
            c"(version 1)(allow default)(deny network*)(deny process-exec)(deny process-fork)";
        let mut error = null_mut();
        if sandbox_init(profile.as_ptr(), 0, &mut error) != 0 {
            if !error.is_null() {
                sandbox_free_error(error);
            }
            libc::close(queue);
            return Err(protection_error("sandbox_init"));
        }
        verify_restrictions().inspect_err(|_| {
            libc::close(queue);
        })?;
        std::thread::Builder::new()
            .name("wallet-parent-watch".into())
            .spawn(move || loop {
                let mut exit: libc::kevent = zeroed();
                let count = libc::kevent(queue, null(), 0, &mut exit, 1, null());
                if count != 0 {
                    libc::_exit(1);
                }
                std::thread::sleep(Duration::from_millis(10));
            })
            .map_err(|_| {
                libc::close(queue);
                "wallet_process_protection_unavailable"
            })?;
    }
    Ok(())
}

fn verify_restrictions() -> Result<()> {
    // Probe real system calls before any key is opened. The operation-query SPI
    // rejects process-exec with EINVAL on current macOS, which is not evidence
    // that exec is allowed or denied. An unexpected successful exec runs only
    // this fixed OS no-op and exits without replying to the parent's Open frame.
    fn denied() -> bool {
        matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM | libc::EACCES)
        )
    }
    unsafe {
        let argv = [c"/usr/bin/true".as_ptr(), null()];
        let env = [null()];
        libc::execve(argv[0], argv.as_ptr(), env.as_ptr());
        if !denied() {
            return Err(protection_error("exec_probe"));
        }
        let child = libc::fork();
        if child == 0 {
            libc::_exit(0);
        }
        if child > 0 {
            libc::waitpid(child, null_mut(), 0);
            return Err(protection_error("fork_allowed"));
        }
        if !denied() {
            return Err(protection_error("fork_probe"));
        }
        for family in [libc::AF_INET, libc::AF_INET6] {
            let socket = libc::socket(family, libc::SOCK_STREAM, 0);
            if socket < 0 {
                if denied() {
                    continue;
                }
                return Err(protection_error("socket_probe"));
            }
            let result = if family == libc::AF_INET {
                let address = libc::sockaddr_in {
                    sin_len: std::mem::size_of::<libc::sockaddr_in>() as u8,
                    sin_family: libc::AF_INET as u8,
                    sin_port: 9u16.to_be(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
                    },
                    sin_zero: [0; 8],
                };
                libc::connect(
                    socket,
                    (&address as *const libc::sockaddr_in).cast(),
                    std::mem::size_of_val(&address) as u32,
                )
            } else {
                let mut address: libc::sockaddr_in6 = zeroed();
                address.sin6_len = std::mem::size_of_val(&address) as u8;
                address.sin6_family = libc::AF_INET6 as u8;
                address.sin6_port = 9u16.to_be();
                address.sin6_addr.s6_addr[15] = 1;
                libc::connect(
                    socket,
                    (&address as *const libc::sockaddr_in6).cast(),
                    std::mem::size_of_val(&address) as u32,
                )
            };
            let blocked = result < 0 && denied();
            libc::close(socket);
            if !blocked {
                return Err(protection_error("network_probe"));
            }
        }
    }
    Ok(())
}

fn close_inherited() -> Result<()> {
    unsafe {
        let needed = libc::proc_pidinfo(libc::getpid(), libc::PROC_PIDLISTFDS, 0, null_mut(), 0);
        if needed <= 0 || needed > 8_000_000 {
            return Err("wallet_process_protection_unavailable".into());
        }
        let count = needed as usize / std::mem::size_of::<libc::proc_fdinfo>() + 16;
        let mut entries: Vec<libc::proc_fdinfo> = (0..count).map(|_| zeroed()).collect();
        let bytes = libc::proc_pidinfo(
            libc::getpid(),
            libc::PROC_PIDLISTFDS,
            0,
            entries.as_mut_ptr().cast(),
            (entries.len() * std::mem::size_of::<libc::proc_fdinfo>()) as i32,
        );
        if bytes <= 0 {
            return Err("wallet_process_protection_unavailable".into());
        }
        for entry in entries
            .iter()
            .take(bytes as usize / std::mem::size_of::<libc::proc_fdinfo>())
        {
            if entry.proc_fd > 2 {
                libc::close(entry.proc_fd);
            }
        }
        Ok(())
    }
}

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
    fn sandbox_check(pid: libc::pid_t, operation: *const c_char, filter: c_int, ...) -> c_int;
}
pub fn enter() -> Result<()> {
    unsafe {
        close_inherited()?;
        let parent = libc::getppid();
        if parent <= 1 {
            return Err("wallet_process_protection_unavailable".into());
        }
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0
            || libc::ptrace(libc::PT_DENY_ATTACH, 0, null_mut(), 0) != 0
        {
            return Err("wallet_process_protection_unavailable".into());
        }
        // Register before applying process-info restrictions, then check for the
        // parent-exit race. A dedicated kernel event kills even a busy KDF worker.
        let queue = libc::kqueue();
        if queue < 0 {
            return Err("wallet_process_protection_unavailable".into());
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
            return Err("wallet_process_protection_unavailable".into());
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
            return Err("wallet_process_protection_unavailable".into());
        }
        for op in [c"network-outbound", c"process-exec", c"process-fork"] {
            if sandbox_check(libc::getpid(), op.as_ptr(), 0) <= 0 {
                libc::close(queue);
                return Err("wallet_process_protection_unavailable".into());
            }
        }
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

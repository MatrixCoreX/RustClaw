use crate::Result;

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub fn enter() -> Result<()> {
    use libc::{sock_filter, sock_fprog};
    fn stmt(code: u16, k: u32) -> sock_filter {
        sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }
    fn jump(k: u32, jt: u8, jf: u8) -> sock_filter {
        sock_filter {
            code: 0x15,
            jt,
            jf,
            k,
        }
    }
    const LOAD: u16 = 0x20;
    const RET: u16 = 0x06;
    const DENY: u32 = libc::SECCOMP_RET_ERRNO | libc::EPERM as u32;
    const ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
    #[cfg(target_arch = "x86_64")]
    let arch = 0xc000003e;
    #[cfg(target_arch = "aarch64")]
    let arch = 0xc00000b7;
    // SAFETY: all prctl/rlimit arguments have the documented scalar/struct types.
    unsafe {
        // Do not retain any inherited sockets or files from the GUI process.
        if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) != 0 {
            return Err("wallet_process_protection_unavailable".into());
        }
        let parent = libc::getppid();
        if parent <= 1
            || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0) != 0
            || libc::getppid() != parent
            || libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0
            || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
        {
            return Err("wallet_process_protection_unavailable".into());
        }
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
            return Err("wallet_process_protection_unavailable".into());
        }
    }
    let mut program = vec![
        stmt(LOAD, 4),
        jump(arch, 1, 0),
        stmt(RET, libc::SECCOMP_RET_KILL_PROCESS),
        stmt(LOAD, 0),
    ];
    #[cfg(target_arch = "x86_64")]
    {
        // Reject x32 syscall encodings as well as the 32-bit audit architecture.
        program.push(sock_filter {
            code: 0x35,
            jt: 0,
            jf: 1,
            k: 0x40000000,
        });
        program.push(stmt(RET, libc::SECCOMP_RET_KILL_PROCESS));
    }
    for syscall in [
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_userfaultfd,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
    ] {
        program.extend([jump(syscall as u32, 0, 1), stmt(RET, DENY)]);
    }
    #[cfg(target_arch = "x86_64")]
    for syscall in [libc::SYS_fork, libc::SYS_vfork] {
        program.extend([jump(syscall as u32, 0, 1), stmt(RET, DENY)]);
    }
    // glibc can fall back from clone3 to the filterable clone flags for threads.
    program.extend([
        jump(libc::SYS_clone3 as u32, 0, 1),
        stmt(RET, libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32),
    ]);
    program.extend([
        jump(libc::SYS_clone as u32, 0, 4),
        stmt(LOAD, 16),
        sock_filter {
            code: 0x45,
            jt: 1,
            jf: 0,
            k: libc::CLONE_THREAD as u32,
        },
        stmt(RET, DENY),
        stmt(RET, ALLOW),
    ]);
    // Local IPC is required by Secret Service. No IP, netlink or packet sockets.
    for syscall in [libc::SYS_socket, libc::SYS_socketpair] {
        program.extend([
            jump(syscall as u32, 0, 4),
            stmt(LOAD, 16),
            jump(libc::AF_UNIX as u32, 1, 0),
            stmt(RET, DENY),
            stmt(RET, ALLOW),
        ]);
    }
    // A compromised worker cannot re-enable dumps or detach from parent death.
    program.extend([
        jump(libc::SYS_prctl as u32, 0, 5),
        stmt(LOAD, 16),
        jump(libc::PR_SET_DUMPABLE as u32, 1, 0),
        jump(libc::PR_SET_PDEATHSIG as u32, 0, 1),
        stmt(RET, DENY),
        stmt(RET, ALLOW),
        stmt(RET, ALLOW),
    ]);
    let filter = sock_fprog {
        len: program.len() as u16,
        filter: program.as_mut_ptr(),
    };
    // SAFETY: filter points to valid BPF instructions for the duration of the call.
    if unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            libc::SECCOMP_FILTER_FLAG_TSYNC,
            &filter,
        )
    } != 0
    {
        return Err("wallet_process_protection_unavailable".into());
    }
    verify()
}
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn verify() -> Result<()> {
    // SAFETY: query-only prctl calls and socket probes send no network traffic.
    unsafe {
        if libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) != 0
            || libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 1
            || libc::prctl(libc::PR_GET_SECCOMP, 0, 0, 0, 0) != 2
        {
            return Err("wallet_process_protection_unavailable".into());
        }
        for family in [libc::AF_INET, libc::AF_INET6, libc::AF_NETLINK] {
            let fd = libc::socket(family, libc::SOCK_STREAM, 0);
            if fd >= 0 {
                libc::close(fd);
                return Err("wallet_process_protection_unavailable".into());
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
                return Err("wallet_process_protection_unavailable".into());
            }
        }
    }
    Ok(())
}
#[cfg(target_os = "macos")]
#[path = "sandbox_macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::enter;
#[cfg(windows)]
#[path = "sandbox_windows.rs"]
mod windows;
#[cfg(windows)]
pub use windows::enter;
#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
pub fn enter() -> Result<()> {
    Err("wallet_process_protection_unavailable".into())
}

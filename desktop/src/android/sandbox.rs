//! Android's private, non-exported service retains ART/Binder descriptors.
//! Only its signing thread gets this extra filter; the application UID/SELinux
//! sandbox and Binder death recipient protect the enclosing service process.
use crate::Result;
pub fn enter() -> Result<()> {
    unsafe {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0
            || libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) != 0
            || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
        {
            return Err("wallet_process_protection_unavailable".into());
        }
    }
    fn stmt(code: u16, k: u32) -> libc::sock_filter {
        libc::sock_filter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }
    fn jump(k: u32, jt: u8, jf: u8) -> libc::sock_filter {
        libc::sock_filter {
            code: 0x15,
            jt,
            jf,
            k,
        }
    }
    #[cfg(target_arch = "aarch64")]
    let arch = 0xc00000b7;
    #[cfg(target_arch = "arm")]
    let arch = 0x40000028;
    #[cfg(target_arch = "x86_64")]
    let arch = 0xc000003e;
    let deny = libc::SECCOMP_RET_ERRNO | libc::EPERM as u32;
    let mut code = vec![
        stmt(0x20, 4),
        jump(arch, 1, 0),
        stmt(6, libc::SECCOMP_RET_KILL_PROCESS),
        stmt(0x20, 0),
    ];
    #[cfg(target_arch = "x86_64")]
    code.extend([
        libc::sock_filter {
            code: 0x35,
            jt: 0,
            jf: 1,
            k: 0x40000000,
        },
        stmt(6, libc::SECCOMP_RET_KILL_PROCESS),
    ]);
    for syscall in [
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
    ] {
        code.extend([jump(syscall as u32, 0, 1), stmt(6, deny)]);
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "arm"))]
    for syscall in [libc::SYS_fork, libc::SYS_vfork] {
        code.extend([jump(syscall as u32, 0, 1), stmt(6, deny)]);
    }
    code.extend([
        jump(libc::SYS_clone3 as u32, 0, 1),
        stmt(6, libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32),
    ]);
    code.extend([
        jump(libc::SYS_clone as u32, 0, 4),
        stmt(0x20, 16),
        libc::sock_filter {
            code: 0x45,
            jt: 1,
            jf: 0,
            k: libc::CLONE_THREAD as u32,
        },
        stmt(6, deny),
        stmt(6, libc::SECCOMP_RET_ALLOW),
    ]);
    for syscall in [libc::SYS_socket, libc::SYS_socketpair] {
        code.extend([
            jump(syscall as u32, 0, 4),
            stmt(0x20, 16),
            jump(libc::AF_UNIX as u32, 1, 0),
            stmt(6, deny),
            stmt(6, libc::SECCOMP_RET_ALLOW),
        ]);
    }
    code.extend([
        jump(libc::SYS_prctl as u32, 0, 4),
        stmt(0x20, 16),
        jump(libc::PR_SET_DUMPABLE as u32, 0, 1),
        stmt(6, deny),
        stmt(6, libc::SECCOMP_RET_ALLOW),
    ]);
    code.push(stmt(6, libc::SECCOMP_RET_ALLOW));
    let filter = libc::sock_fprog {
        len: code.len() as u16,
        filter: code.as_mut_ptr(),
    };
    // Do not apply TSYNC to ART's threads: Android owns their existing filters.
    if unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &filter) } != 0 {
        return Err("wallet_process_protection_unavailable".into());
    }
    unsafe {
        if libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) != 0
            || libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) != 1
        {
            return Err("wallet_process_protection_unavailable".into());
        }
        for family in [libc::AF_INET, libc::AF_INET6] {
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

use std::collections::BTreeSet;
use std::io::Read;

use super::super::super::owner_privacy::OwnerProcessPrivacy;
use super::super::ScopeFailure;

pub(super) const MAX_OWNED_CHILDREN: usize = 16_384;
const MAX_CHILD_RECORD_BYTES: u64 = 256 * 1024;
const MAX_OWNER_DESCRIPTORS: usize = 16_384;

pub(super) struct OwnerStamp {
    pid: u32,
    tid: libc::pid_t,
    privacy: OwnerProcessPrivacy,
}

impl OwnerStamp {
    pub(super) fn validate(&self) -> Result<(), ScopeFailure> {
        self.privacy.validate()?;
        if std::process::id() != self.pid
            || unsafe { libc::syscall(libc::SYS_gettid) } != libc::c_long::from(self.tid)
        {
            return Err(ScopeFailure::OwnershipLost);
        }
        let mut enabled = 0_i32;
        if unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut enabled, 0, 0, 0) } != 0
            || enabled != 1
        {
            return Err(ScopeFailure::OwnershipLost);
        }
        validate_sigchld()
    }
}

fn validate_sigchld() -> Result<(), ScopeFailure> {
    let mut disposition = unsafe { std::mem::zeroed::<libc::sigaction>() };
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut disposition) } != 0 {
        return Err(ScopeFailure::ProbeUnavailable);
    }
    if disposition.sa_sigaction != libc::SIG_DFL || disposition.sa_flags & libc::SA_NOCLDWAIT != 0 {
        return Err(ScopeFailure::InvalidSignalDisposition);
    }
    Ok(())
}

pub(super) fn enter_owner_process() -> Result<OwnerStamp, ScopeFailure> {
    let privacy = OwnerProcessPrivacy::admit_current()?;
    validate_sigchld()?;
    let mut enabled = 0_i32;
    if unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut enabled, 0, 0, 0) } != 0 {
        return Err(ScopeFailure::Unsupported);
    }
    if enabled != 0 {
        return Err(ScopeFailure::OwnerContextInUse);
    }
    // WNOWAIT is essential here: checking admission must never reap a child
    // that belongs to some other owner. Feature-detect __WALL support too.
    let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    if unsafe {
        libc::waitid(
            libc::P_ALL,
            0,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT | libc::__WALL,
        )
    } == 0
    {
        return Err(ScopeFailure::OwnerContextInUse);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ECHILD) => {}
        Some(libc::EINVAL) => return Err(ScopeFailure::Unsupported),
        _ => return Err(ScopeFailure::ProbeUnavailable),
    }
    validate_descriptor_inheritance()?;
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        return Err(ScopeFailure::Unsupported);
    }
    Ok(OwnerStamp {
        pid: std::process::id(),
        tid: unsafe { libc::syscall(libc::SYS_gettid) } as libc::pid_t,
        privacy,
    })
}

fn validate_descriptor_inheritance() -> Result<(), ScopeFailure> {
    // Enumerate the actual table, not 0..RLIMIT_NOFILE: existing high FDs remain
    // open after lowering the soft limit. Never close or rewrite caller FDs.
    // This is pre-fork and relies on the exclusive helper contract; the only
    // later descriptors are trusted Command stdio/error pipes with CLOEXEC.
    let descriptors =
        std::fs::read_dir("/proc/self/fd").map_err(|_| ScopeFailure::ProbeUnavailable)?;
    for (index, entry) in descriptors.enumerate() {
        if index >= MAX_OWNER_DESCRIPTORS {
            return Err(ScopeFailure::LimitExceeded);
        }
        let entry = entry.map_err(|_| ScopeFailure::ProbeUnavailable)?;
        let fd = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<i32>().ok())
            .filter(|fd| *fd >= 0)
            .ok_or(ScopeFailure::ProbeUnavailable)?;
        if fd <= 2 {
            continue;
        }
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags < 0 {
            return Err(ScopeFailure::ProbeUnavailable);
        }
        if flags & libc::FD_CLOEXEC == 0 {
            return Err(ScopeFailure::InvalidDescriptor);
        }
    }
    Ok(())
}

// This is bounded discovery for signals, never completion evidence. Every PID
// is checked again with waitpid before a signal; completion uses kernel ECHILD.
pub(super) fn child_candidates() -> Result<BTreeSet<u32>, ScopeFailure> {
    let tasks = std::fs::read_dir("/proc/self/task").map_err(|_| ScopeFailure::ProbeUnavailable)?;
    let mut children = BTreeSet::new();
    for (count, task) in tasks.enumerate() {
        if count >= 32 {
            return Err(ScopeFailure::LimitExceeded);
        }
        let task = task.map_err(|_| ScopeFailure::ProbeUnavailable)?;
        let path = task.path().join("children");
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(ScopeFailure::ProbeUnavailable),
        };
        let mut bytes = Vec::new();
        file.take(MAX_CHILD_RECORD_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ScopeFailure::ProbeUnavailable)?;
        if bytes.len() as u64 > MAX_CHILD_RECORD_BYTES {
            return Err(ScopeFailure::LimitExceeded);
        }
        for pid in bytes
            .split(u8::is_ascii_whitespace)
            .filter(|part| !part.is_empty())
        {
            let pid = std::str::from_utf8(pid)
                .ok()
                .and_then(|pid| pid.parse::<u32>().ok())
                .filter(|pid| *pid > 1 && *pid <= i32::MAX as u32)
                .ok_or(ScopeFailure::ProbeUnavailable)?;
            children.insert(pid);
            if children.len() > MAX_OWNED_CHILDREN {
                return Err(ScopeFailure::LimitExceeded);
            }
        }
    }
    Ok(children)
}

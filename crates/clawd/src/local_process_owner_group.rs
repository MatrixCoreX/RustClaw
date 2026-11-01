use std::process::{Child, ChildStdin, Command};
use std::time::{Duration, Instant};

use super::exit::ChildExit;
use super::instance::{ProcessProbeFailure, ProcessRunState};
use super::members::group_members;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupFailure {
    SpawnFailed,
    OwnershipLost,
    ProbeUnavailable,
    SignalFailed,
    AlreadyReaped,
    Unsupported,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum OwnedSignal {
    Interrupt,
    Terminate,
    Kill,
}

impl OwnedSignal {
    fn raw(self) -> i32 {
        match self {
            Self::Interrupt => libc::SIGINT,
            Self::Terminate => libc::SIGTERM,
            Self::Kill => libc::SIGKILL,
        }
    }
}

/// Only a dedicated single-threaded owner may hold this value. It must not
/// install an ambient child reaper. No constructor accepts a PID or metadata.
pub(crate) struct OwnedChildGroup {
    child: Child,
    reaped: bool,
}

impl OwnedChildGroup {
    pub(crate) fn spawn(command: &mut Command) -> Result<Self, GroupFailure> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            use std::os::unix::process::CommandExt;
            let child = command
                .process_group(0)
                .spawn()
                .map_err(|_| GroupFailure::SpawnFailed)?;
            Ok(Self {
                child,
                reaped: false,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = command;
            Err(GroupFailure::Unsupported)
        }
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub(crate) fn poll_exit(&mut self) -> Result<Option<ChildExit>, GroupFailure> {
        if self.reaped {
            return Err(GroupFailure::AlreadyReaped);
        }
        waitable_exit(self.id())
    }

    pub(crate) fn signal(&mut self, signal: OwnedSignal) -> Result<(), GroupFailure> {
        self.poll_exit()?;
        let signal = signal.raw();
        let original_membership = unsafe { libc::getpgid(self.id() as i32) };
        if original_membership < 0
            && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        {
            return Err(GroupFailure::ProbeUnavailable);
        }
        // The child has not been reaped, so its PID also pins the original
        // process-group number. Never replace this with a metadata lookup.
        let group_result = unsafe { libc::kill(-(self.id() as i32), signal) };
        if group_result != 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        {
            return Err(GroupFailure::SignalFailed);
        }
        // An owned child may have left its original group; signal only that
        // child as well, not whichever group it has joined.
        if original_membership == self.id() as i32 && group_result == 0 {
            return Ok(());
        }
        if unsafe { libc::kill(self.id() as i32, signal) } == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            Ok(())
        } else {
            Err(GroupFailure::SignalFailed)
        }
    }

    // Original-group prototype only. An empty enumeration is not atomic
    // whole-job completion proof; production ownership must use a real scope.
    pub(crate) fn try_finish(&mut self) -> Result<Option<ChildExit>, GroupFailure> {
        let Some(exit) = self.poll_exit()? else {
            return Ok(None);
        };
        let members = group_members(self.id()).map_err(|error| match error {
            ProcessProbeFailure::Unsupported => GroupFailure::Unsupported,
            _ => GroupFailure::ProbeUnavailable,
        })?;
        if members
            .iter()
            .any(|member| member.birth.pid != self.id() && member.state == ProcessRunState::Active)
        {
            return Ok(None);
        }
        self.reaped = true;
        self.child.wait().map_err(|_| GroupFailure::OwnershipLost)?;
        Ok(Some(exit))
    }
}

fn waitable_exit(pid: u32) -> Result<Option<ChildExit>, GroupFailure> {
    if pid <= 1 || pid > i32::MAX as u32 {
        return Err(GroupFailure::OwnershipLost);
    }
    let mut info = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
                GroupFailure::OwnershipLost
            } else {
                GroupFailure::ProbeUnavailable
            },
        );
    }
    let observed_pid = unsafe { info.si_pid() };
    if observed_pid == 0 {
        return Ok(None);
    }
    if observed_pid as u32 != pid {
        return Err(GroupFailure::OwnershipLost);
    }
    let status = unsafe { info.si_status() };
    decode_exit(info.si_code, status)
}

fn decode_exit(kind: i32, status: i32) -> Result<Option<ChildExit>, GroupFailure> {
    match kind {
        libc::CLD_EXITED => Ok(Some(ChildExit {
            // Darwin waitid may include extra high bits which a subsequent
            // WNOWAIT observation clears. Keep the normal Unix exit code.
            code: Some(status & 0xff),
            signal: None,
        })),
        libc::CLD_KILLED | libc::CLD_DUMPED if status > 0 => Ok(Some(ChildExit {
            code: None,
            signal: Some(status),
        })),
        _ => Err(GroupFailure::ProbeUnavailable),
    }
}

impl Drop for OwnedChildGroup {
    fn drop(&mut self) {
        if self.reaped || self.poll_exit().is_err() {
            return;
        }
        let _ = self.signal(OwnedSignal::Kill);
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                self.reaped = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
#[path = "local_process_owner_group_tests.rs"]
mod tests;

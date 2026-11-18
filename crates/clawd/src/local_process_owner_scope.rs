#[cfg(test)]
use std::process::ChildStdin;
use std::process::Command;
use std::time::Duration;

use super::exit::ChildExit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeFailure {
    Unsupported,
    OwnerContextInUse,
    InvalidSignalDisposition,
    InvalidDescriptor,
    SpawnFailed,
    ProbeUnavailable,
    OwnershipLost,
    SignalFailed,
    LimitExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScopeSnapshot {
    pub(crate) root_exit: Option<ChildExit>,
    pub(crate) complete: bool,
    pub(crate) cancel_requested: bool,
}

pub(crate) struct OwnedProcessScope {
    #[cfg(target_os = "linux")]
    scope: linux::LinuxOwnedScope,
}

impl OwnedProcessScope {
    /// # Safety
    /// The caller must be an independent, one-job owner process with exclusive
    /// control over child spawning/reaping and no ambient SIGCHLD reaper.
    /// Preexisting children are refused without being reaped or signalled.
    /// The helper must have entered process privacy before any private input;
    /// scope admission validates it and never repairs a missing/lost guard.
    /// This process must exit after the scope is retired or initialization fails.
    /// Never call this inside the daemon or a parent test runner.
    pub(crate) unsafe fn spawn(
        command: &mut Command,
        grace: Duration,
    ) -> Result<Self, ScopeFailure> {
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                scope: unsafe { linux::LinuxOwnedScope::spawn(command, grace)? },
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (command, grace);
            Err(ScopeFailure::Unsupported)
        }
    }

    #[cfg(test)]
    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        #[cfg(target_os = "linux")]
        {
            self.scope.take_stdin()
        }
        #[cfg(not(target_os = "linux"))]
        {
            None
        }
    }

    pub(crate) fn poll(&mut self) -> Result<ScopeSnapshot, ScopeFailure> {
        #[cfg(target_os = "linux")]
        {
            self.scope.poll()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(ScopeFailure::Unsupported)
        }
    }

    pub(crate) fn cancel(&mut self) -> Result<ScopeSnapshot, ScopeFailure> {
        #[cfg(target_os = "linux")]
        {
            self.scope.cancel()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(ScopeFailure::Unsupported)
        }
    }
}

#[cfg(target_os = "linux")]
#[path = "local_process_scope_linux.rs"]
mod linux;

#[cfg(test)]
#[path = "local_process_owner_scope_tests.rs"]
pub(super) mod tests;

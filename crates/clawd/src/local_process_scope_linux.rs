use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::os::unix::process::{CommandExt, ExitStatusExt};
#[cfg(test)]
use std::process::ChildStdin;
use std::process::{Child, Command, ExitStatus};
use std::rc::Rc;
use std::time::{Duration, Instant};

use super::super::exit::ChildExit;
use super::{ScopeFailure, ScopeSnapshot};

#[path = "local_process_scope_linux_children.rs"]
mod children;

const REAP_BUDGET: usize = 256;

pub(super) struct LinuxOwnedScope {
    root: Child,
    stamp: children::OwnerStamp,
    snapshot: ScopeSnapshot,
    cancellation: Option<Instant>,
    grace: Duration,
    sent: BTreeMap<u32, i32>,
    // The same exclusive waiter must validate a child and send its signal.
    _not_send: PhantomData<Rc<()>>,
}

impl LinuxOwnedScope {
    pub(super) unsafe fn spawn(
        command: &mut Command,
        grace: Duration,
    ) -> Result<Self, ScopeFailure> {
        let stamp = children::enter_owner_process()?;
        let root = command
            .process_group(0)
            .spawn()
            .map_err(|_| ScopeFailure::SpawnFailed)?;
        Ok(Self {
            root,
            stamp,
            grace,
            snapshot: ScopeSnapshot {
                root_exit: None,
                complete: false,
                cancel_requested: false,
            },
            cancellation: None,
            sent: BTreeMap::new(),
            _not_send: PhantomData,
        })
    }

    #[cfg(test)]
    pub(super) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.root.stdin.take()
    }

    pub(super) fn cancel(&mut self) -> Result<ScopeSnapshot, ScopeFailure> {
        let observed = self.poll();
        self.cancel_observed(observed)
    }

    fn cancel_observed(
        &mut self,
        observed: Result<ScopeSnapshot, ScopeFailure>,
    ) -> Result<ScopeSnapshot, ScopeFailure> {
        match observed {
            Ok(snapshot) if snapshot.complete => return Ok(snapshot),
            Ok(_) | Err(ScopeFailure::ProbeUnavailable) => {}
            Err(error) => return Err(error),
        }
        if self.cancellation.is_none() {
            self.cancellation = Some(Instant::now());
            self.snapshot.cancel_requested = true;
        }
        self.poll()
    }

    pub(super) fn poll(&mut self) -> Result<ScopeSnapshot, ScopeFailure> {
        self.stamp.validate()?;
        if self.snapshot.complete {
            return Ok(self.snapshot);
        }
        let observation = self.reap_exits();
        self.poll_observed(observation)
    }

    fn poll_observed(
        &mut self,
        observation: Result<(), ScopeFailure>,
    ) -> Result<ScopeSnapshot, ScopeFailure> {
        match observation {
            Err(ScopeFailure::ProbeUnavailable) | Ok(()) => {}
            Err(error) => return Err(error),
        }
        if self.snapshot.complete {
            return Ok(self.snapshot);
        }
        if self.cancellation.is_some() {
            self.signal_current_children()?;
        }
        observation.map(|()| self.snapshot)
    }

    fn record_exit(&mut self, pid: u32, status: i32) -> bool {
        let status = ExitStatus::from_raw(status);
        if status.code().is_none() && status.signal().is_none() {
            return false;
        }
        // Removal happens on every reap, before this PID can name a new child.
        self.sent.remove(&pid);
        self.snapshot.root_exit = root_exit_after_reap(
            self.snapshot.root_exit,
            self.root.id(),
            pid,
            ChildExit {
                code: status.code(),
                signal: status.signal(),
            },
        );
        true
    }

    fn reap_exits(&mut self) -> Result<(), ScopeFailure> {
        for _ in 0..REAP_BUDGET {
            let mut status = 0;
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG | libc::__WALL) };
            if pid > 0 {
                self.record_exit(pid as u32, status);
            } else if pid == 0 {
                return Ok(());
            } else {
                match std::io::Error::last_os_error().raw_os_error() {
                    Some(libc::ECHILD) if self.snapshot.root_exit.is_some() => {
                        self.snapshot.complete = true;
                        return Ok(());
                    }
                    Some(libc::ECHILD) => return Err(ScopeFailure::OwnershipLost),
                    Some(libc::EINTR) => continue,
                    _ => return Err(ScopeFailure::ProbeUnavailable),
                }
            }
        }
        Ok(())
    }

    fn signal_current_children(&mut self) -> Result<(), ScopeFailure> {
        let discovered = children::child_candidates();
        let mut candidates: BTreeSet<u32> = self.sent.keys().copied().collect();
        if self.snapshot.root_exit.is_none() {
            candidates.insert(self.root.id());
        }
        if let Ok(discovered) = &discovered {
            candidates.extend(discovered);
        }
        let signal = if self
            .cancellation
            .is_some_and(|at| at.elapsed() >= self.grace)
        {
            libc::SIGKILL
        } else {
            libc::SIGTERM
        };
        for pid in candidates {
            if self.sent.get(&pid) == Some(&signal) {
                continue;
            }
            if !self.live_owned_child(pid)? {
                continue;
            }
            if self.sent.len() == children::MAX_OWNED_CHILDREN && !self.sent.contains_key(&pid) {
                return Err(ScopeFailure::LimitExceeded);
            }
            // No other waiter may run between the child check and this call.
            // The live/unreaped child pins its own PID; never signal its group.
            if unsafe { libc::kill(pid as libc::pid_t, signal) } != 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            {
                return Err(ScopeFailure::SignalFailed);
            }
            self.sent.insert(pid, signal);
        }
        discovered.map(|_| ())
    }

    fn live_owned_child(&mut self, pid: u32) -> Result<bool, ScopeFailure> {
        if pid <= 1 || pid > i32::MAX as u32 {
            return Err(ScopeFailure::OwnershipLost);
        }
        let mut status = 0;
        let observed = unsafe {
            libc::waitpid(
                pid as libc::pid_t,
                &mut status,
                libc::WNOHANG | libc::__WALL,
            )
        };
        if observed == 0 {
            Ok(true)
        } else if observed == pid as libc::pid_t {
            Ok(!self.record_exit(pid, status))
        } else {
            Err(
                if std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
                    ScopeFailure::OwnershipLost
                } else {
                    ScopeFailure::ProbeUnavailable
                },
            )
        }
    }
}

fn root_exit_after_reap(
    current: Option<ChildExit>,
    root_pid: u32,
    reaped_pid: u32,
    exit: ChildExit,
) -> Option<ChildExit> {
    current.or((root_pid == reaped_pid).then_some(exit))
}

impl Drop for LinuxOwnedScope {
    fn drop(&mut self) {
        if self.snapshot.complete || self.stamp.validate().is_err() {
            return;
        }
        self.cancellation.get_or_insert_with(Instant::now);
        self.grace = Duration::ZERO;
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            match self.poll() {
                Ok(snapshot) if snapshot.complete => break,
                Err(ScopeFailure::OwnershipLost | ScopeFailure::InvalidSignalDisposition) => break,
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
#[path = "local_process_scope_linux_state_tests.rs"]
pub(super) mod tests;

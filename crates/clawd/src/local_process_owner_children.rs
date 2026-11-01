use std::io;
use std::process::{Child, ExitStatus};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub(super) struct OwnerChildReaper(mpsc::Sender<PendingChild>);

// Observation of one immediate child, never authenticated scope completion.
// The waiter owns the actual Child independently of every observer's lifetime.
pub(crate) struct HostOwnerChild {
    pid: u32,
    observation: Arc<Mutex<Observation>>,
}

enum ChildState {
    Waiting,
    Unavailable(io::ErrorKind, Option<i32>),
    Reaped(ExitStatus),
}

struct Observation {
    state: ChildState,
    #[cfg(test)]
    crash_request: Option<mpsc::Sender<io::Result<()>>>,
}

struct PendingChild {
    child: Child,
    observation: Arc<Mutex<Observation>>,
}

impl OwnerChildReaper {
    #[cfg(test)]
    pub(super) fn closed_for_test() -> Self {
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        Self(sender)
    }

    pub(super) fn new() -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        // One waiter per pinned host, not one thread per long-running child.
        let worker = std::thread::Builder::new()
            .name("owner-child-reaper".into())
            .spawn(move || drive(receiver))?;
        drop(worker);
        Ok(Self(sender))
    }

    pub(super) fn retain(&self, child: Child) -> Result<HostOwnerChild, Child> {
        let pid = child.id();
        let observation = Arc::new(Mutex::new(Observation {
            state: ChildState::Waiting,
            #[cfg(test)]
            crash_request: None,
        }));
        let pending = PendingChild {
            child,
            observation: Arc::clone(&observation),
        };
        match self.0.send(pending) {
            Ok(()) => Ok(HostOwnerChild { pid, observation }),
            Err(error) => Err(error.0.child),
        }
    }
}

impl HostOwnerChild {
    pub(crate) fn id(&self) -> u32 {
        self.pid
    }

    pub(crate) fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        let observed = self
            .observation
            .lock()
            .map_err(|_| io::Error::other("owner_child_observation_unavailable"))?;
        match &observed.state {
            ChildState::Waiting => Ok(None),
            ChildState::Unavailable(kind, errno) => Err(errno
                .map(io::Error::from_raw_os_error)
                .unwrap_or_else(|| io::Error::from(*kind))),
            ChildState::Reaped(status) => Ok(Some(*status)),
        }
    }

    #[cfg(test)]
    pub(crate) fn kill(&self) -> io::Result<()> {
        let (reply, response) = mpsc::channel();
        {
            let mut observed = self
                .observation
                .lock()
                .map_err(|_| io::Error::other("owner_child_observation_unavailable"))?;
            if !matches!(observed.state, ChildState::Waiting) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            observed.crash_request = Some(reply);
        }
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
    }
}

fn poll(pending: &mut PendingChild) -> bool {
    let mut observed = pending
        .observation
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    #[cfg(test)]
    if let Some(reply) = observed.crash_request.take() {
        let result = match pending.child.try_wait() {
            Ok(None) => pending.child.kill(),
            Ok(Some(_)) => Err(io::ErrorKind::InvalidInput.into()),
            Err(error) => Err(error),
        };
        let _ = reply.send(result);
    }
    match pending.child.try_wait() {
        Ok(Some(status)) => {
            observed.state = ChildState::Reaped(status);
            false
        }
        Ok(None) => {
            observed.state = ChildState::Waiting;
            true
        }
        Err(error) => {
            let errno = error.raw_os_error();
            observed.state = ChildState::Unavailable(error.kind(), errno);
            // Losing the kernel child reference is unknown, not a successful
            // reap. Stop probing that saved PID; no signal or adoption follows.
            errno != Some(libc::ECHILD)
        }
    }
}

fn drive(receiver: mpsc::Receiver<PendingChild>) {
    let mut children = Vec::new();
    let mut disconnected = false;
    loop {
        children.retain_mut(poll);
        if disconnected {
            if children.is_empty() {
                return;
            }
            std::thread::sleep(POLL_INTERVAL);
            continue;
        }
        let message = if children.is_empty() {
            receiver
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        } else {
            receiver.recv_timeout(POLL_INTERVAL)
        };
        match message {
            Ok(child) => children.push(child),
            Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
#[path = "local_process_owner_children_tests.rs"]
mod tests;

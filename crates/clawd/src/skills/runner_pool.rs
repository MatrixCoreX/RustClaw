use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStderr, ChildStdin, Command};
use tokio_util::codec::{FramedRead, LinesCodec};

use claw_core::host_resources::HostResourceSnapshot;
use claw_core::host_resources::ResourcePressureState;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct WarmRunnerKey {
    pub(crate) scope_token: String,
    pub(crate) version_pin: skill_sdk::SkillVersionPin,
    pub(crate) admission_binding: Option<crate::skill_admission::AdmissionExecutionBinding>,
    pub(crate) registry_generation: u64,
    pub(crate) registry_generation_digest: Option<String>,
    pub(crate) base_registry_digest: Option<String>,
    pub(crate) overlay_generation_digest: Option<String>,
    pub(crate) sandbox_backend: String,
    pub(crate) timeout_seconds: u64,
    pub(crate) memory_reservation_mib: u64,
}

pub(crate) struct WarmRunnerProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    pub(crate) records: FramedRead<tokio::process::ChildStdout, LinesCodec>,
    stderr: Option<ChildStderr>,
    last_used: Instant,
}

impl WarmRunnerProcess {
    pub(crate) fn spawn(mut command: Command) -> Result<Self, std::io::Error> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("piped runner stdin");
        let stdout = child.stdout.take().expect("piped runner stdout");
        let stderr = child.stderr.take();
        Ok(Self {
            child,
            stdin: Some(stdin),
            records: FramedRead::new(
                stdout,
                LinesCodec::new_with_max_length(skill_sdk::MAX_PROTOCOL_LINE_BYTES),
            ),
            stderr,
            last_used: Instant::now(),
        })
    }

    pub(crate) async fn send(&mut self, request: &str) -> Result<(), std::io::Error> {
        let stdin = self.stdin.as_mut().expect("runner stdin available");
        stdin.write_all(request.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await
    }

    pub(crate) fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.stderr.take()
    }

    pub(crate) fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub(crate) fn resident_memory_mib(&self) -> Option<u64> {
        process_tree_resident_memory_mib(self.child.id()?)
    }

    pub(crate) async fn shutdown(&mut self) -> Result<(), std::io::Error> {
        self.stdin.take();
        match tokio::time::timeout(Duration::from_secs(1), self.child.wait()).await {
            Ok(result) => {
                result?;
            }
            Err(_) => {
                let _ = self.child.kill().await;
                self.child.wait().await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn kill_and_wait(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

impl Drop for WarmRunnerProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

struct IdleRunner {
    process: WarmRunnerProcess,
}

pub(crate) enum WarmPoolCheckout {
    Reused(WarmRunnerProcess, u64),
    Spawn(u64),
    Fallback(&'static str),
}

pub(crate) struct WarmRunnerPool {
    enabled: bool,
    max_idle_per_scope: usize,
    min_available_memory_mib: u64,
    idle_timeout: Duration,
    epoch: AtomicU64,
    idle: Mutex<HashMap<WarmRunnerKey, Vec<IdleRunner>>>,
    resource_broker: crate::resource_scheduler::ResourceBroker,
}

impl WarmRunnerPool {
    pub(crate) fn new(
        enabled: bool,
        max_idle_per_scope: usize,
        min_available_memory_mib: u64,
        idle_timeout_seconds: u64,
    ) -> Self {
        Self::new_with_resource_broker(
            enabled,
            max_idle_per_scope,
            min_available_memory_mib,
            idle_timeout_seconds,
            crate::resource_scheduler::ResourceBroker::default(),
        )
    }

    pub(crate) fn new_with_resource_broker(
        enabled: bool,
        max_idle_per_scope: usize,
        min_available_memory_mib: u64,
        idle_timeout_seconds: u64,
        resource_broker: crate::resource_scheduler::ResourceBroker,
    ) -> Self {
        Self {
            enabled,
            max_idle_per_scope: max_idle_per_scope.min(64),
            min_available_memory_mib,
            idle_timeout: Duration::from_secs(idle_timeout_seconds.clamp(1, 3_600)),
            epoch: AtomicU64::new(1),
            idle: Mutex::new(HashMap::new()),
            resource_broker,
        }
    }

    pub(crate) fn checkout(&self, key: &WarmRunnerKey) -> WarmPoolCheckout {
        if !self.enabled || self.max_idle_per_scope == 0 {
            return WarmPoolCheckout::Fallback("warm_pool_disabled");
        }
        if HostResourceSnapshot::collect()
            .available_memory_mib()
            .is_some_and(|value| value < self.min_available_memory_mib)
        {
            self.invalidate_all();
            return WarmPoolCheckout::Fallback("warm_pool_low_memory");
        }
        let mut idle = self.idle.lock().unwrap();
        idle.retain(|candidate, runners| {
            if candidate.scope_token == key.scope_token && candidate != key {
                return false;
            }
            runners.retain_mut(|runner| {
                runner.process.last_used.elapsed() <= self.idle_timeout && runner.process.is_alive()
            });
            !runners.is_empty()
        });
        while let Some(runner) = idle.get_mut(key).and_then(Vec::pop) {
            if runner.process.last_used.elapsed() <= self.idle_timeout {
                return WarmPoolCheckout::Reused(
                    runner.process,
                    self.epoch.load(Ordering::Acquire),
                );
            }
        }
        WarmPoolCheckout::Spawn(self.epoch.load(Ordering::Acquire))
    }

    pub(crate) fn checkin(
        &self,
        key: WarmRunnerKey,
        checkout_epoch: u64,
        mut process: WarmRunnerProcess,
    ) {
        if !self.enabled || self.max_idle_per_scope == 0 || !process.is_alive() {
            schedule_runner_reap(process);
            return;
        }
        let pressure_state = self.resource_broker.pressure_state();
        if matches!(
            pressure_state,
            ResourcePressureState::Constrained | ResourcePressureState::Critical
        ) {
            schedule_runner_reap(process);
            return;
        }
        if process
            .resident_memory_mib()
            .is_some_and(|rss| rss > key.memory_reservation_mib)
        {
            schedule_runner_reap(process);
            return;
        }
        process.last_used = Instant::now();
        let mut idle = self.idle.lock().unwrap();
        if checkout_epoch != self.epoch.load(Ordering::Acquire) {
            drop(idle);
            schedule_runner_reap(process);
            return;
        }
        let runners = idle.entry(key).or_default();
        if runners.len() < self.max_idle_per_scope {
            runners.push(IdleRunner { process });
        } else {
            drop(idle);
            schedule_runner_reap(process);
        }
    }

    pub(crate) fn invalidate_all(&self) {
        let processes = {
            let mut idle = self.idle.lock().unwrap();
            self.epoch.fetch_add(1, Ordering::AcqRel);
            idle.drain()
                .flat_map(|(_, runners)| runners.into_iter().map(|runner| runner.process))
                .collect::<Vec<_>>()
        };
        for process in processes {
            schedule_runner_reap(process);
        }
    }

    #[cfg(test)]
    pub(crate) fn idle_count(&self) -> usize {
        self.idle.lock().unwrap().values().map(Vec::len).sum()
    }
}

fn schedule_runner_reap(mut process: WarmRunnerProcess) {
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => {
            runtime.spawn(async move {
                process.kill_and_wait().await;
            });
        }
        Err(_) => {
            let _ = process.child.start_kill();
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_process_resident_and_swap_kib(pid: u32) -> Option<u64> {
    if let Ok(rollup) = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")) {
        let mut pss_kib = None;
        let mut swap_pss_kib = 0_u64;
        for line in rollup.lines() {
            let value = |prefix: &str| {
                line.strip_prefix(prefix)
                    .and_then(|raw| raw.split_whitespace().next())
                    .and_then(|raw| raw.parse::<u64>().ok())
            };
            if let Some(value) = value("Pss:") {
                pss_kib = Some(value);
            } else if let Some(value) = value("SwapPss:") {
                swap_pss_kib = value;
            }
        }
        if let Some(pss_kib) = pss_kib {
            return Some(pss_kib.saturating_add(swap_pss_kib));
        }
    }
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn process_tree_resident_memory_mib(pid: u32) -> Option<u64> {
    let mut pending = vec![pid];
    let mut visited = std::collections::HashSet::new();
    let mut total_kib = 0_u64;
    let mut observed = false;
    while let Some(current) = pending.pop() {
        if !visited.insert(current) {
            continue;
        }
        if let Some(kib) = linux_process_resident_and_swap_kib(current) {
            total_kib = total_kib.saturating_add(kib);
            observed = true;
        }
        if let Ok(children) =
            std::fs::read_to_string(format!("/proc/{current}/task/{current}/children"))
        {
            pending.extend(
                children
                    .split_whitespace()
                    .filter_map(|value| value.parse::<u32>().ok()),
            );
        }
    }
    observed.then(|| total_kib.div_ceil(1024))
}

#[cfg(target_os = "macos")]
pub(crate) fn process_tree_resident_memory_mib(pid: u32) -> Option<u64> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,rss="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let rows = raw
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            Some((
                fields.next()?.parse::<u32>().ok()?,
                fields.next()?.parse::<u32>().ok()?,
                fields.next()?.parse::<u64>().ok()?,
            ))
        })
        .collect::<Vec<_>>();
    let mut descendants = std::collections::HashSet::from([pid]);
    loop {
        let before = descendants.len();
        for (child, parent, _) in &rows {
            if descendants.contains(parent) {
                descendants.insert(*child);
            }
        }
        if descendants.len() == before {
            break;
        }
    }
    let total_kib = rows
        .iter()
        .filter(|(process, _, _)| descendants.contains(process))
        .map(|(_, _, rss)| *rss)
        .sum::<u64>();
    (total_kib > 0).then(|| total_kib.div_ceil(1024))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn process_tree_resident_memory_mib(_pid: u32) -> Option<u64> {
    None
}

impl Default for WarmRunnerPool {
    fn default() -> Self {
        Self::new(false, 1, 512, 60)
    }
}

#[cfg(test)]
#[path = "runner_pool_tests.rs"]
mod tests;

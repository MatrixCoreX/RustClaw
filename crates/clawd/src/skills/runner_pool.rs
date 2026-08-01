use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStderr, ChildStdin, Command};
use tokio_util::codec::{FramedRead, LinesCodec};

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
}

impl WarmRunnerPool {
    pub(crate) fn new(
        enabled: bool,
        max_idle_per_scope: usize,
        min_available_memory_mib: u64,
        idle_timeout_seconds: u64,
    ) -> Self {
        Self {
            enabled,
            max_idle_per_scope: max_idle_per_scope.min(64),
            min_available_memory_mib,
            idle_timeout: Duration::from_secs(idle_timeout_seconds.clamp(1, 3_600)),
            epoch: AtomicU64::new(1),
            idle: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn checkout(&self, key: &WarmRunnerKey) -> WarmPoolCheckout {
        if !self.enabled || self.max_idle_per_scope == 0 {
            return WarmPoolCheckout::Fallback("warm_pool_disabled");
        }
        if available_memory_mib().is_some_and(|value| value < self.min_available_memory_mib) {
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
            return;
        }
        process.last_used = Instant::now();
        let mut idle = self.idle.lock().unwrap();
        if checkout_epoch != self.epoch.load(Ordering::Acquire) {
            return;
        }
        let runners = idle.entry(key).or_default();
        if runners.len() < self.max_idle_per_scope {
            runners.push(IdleRunner { process });
        }
    }

    pub(crate) fn invalidate_all(&self) {
        let mut idle = self.idle.lock().unwrap();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        idle.clear();
    }

    #[cfg(test)]
    pub(crate) fn idle_count(&self) -> usize {
        self.idle.lock().unwrap().values().map(Vec::len).sum()
    }
}

impl Default for WarmRunnerPool {
    fn default() -> Self {
        Self::new(false, 1, 512, 60)
    }
}

#[cfg(target_os = "linux")]
fn available_memory_mib() -> Option<u64> {
    let raw = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kib = raw
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    Some(kib / 1024)
}

#[cfg(not(target_os = "linux"))]
fn available_memory_mib() -> Option<u64> {
    None
}

#[cfg(test)]
#[path = "runner_pool_tests.rs"]
mod tests;

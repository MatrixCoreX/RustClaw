use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use claw_core::types::WorkerSupervisionHealth;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct WorkerMonitor {
    pub(crate) alive: AtomicBool,
    pub(crate) stalled: AtomicBool,
    pub(crate) restarts: AtomicU64,
    last_pulse: Mutex<Instant>,
    tick_started: Mutex<Instant>,
    task: Mutex<Option<(String, Instant)>>,
}

impl WorkerMonitor {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            alive: AtomicBool::new(false),
            stalled: AtomicBool::new(false),
            restarts: AtomicU64::new(0),
            last_pulse: Mutex::new(Instant::now()),
            tick_started: Mutex::new(Instant::now()),
            task: Mutex::new(None),
        })
    }

    pub(crate) fn pulse(&self) {
        *locked(&self.last_pulse) = Instant::now();
        self.alive.store(true, Ordering::Release);
    }

    pub(crate) fn begin_tick(&self) {
        *locked(&self.task) = None;
        *locked(&self.tick_started) = Instant::now();
        self.stalled.store(false, Ordering::Release);
        self.pulse();
    }

    pub(crate) fn execution_heartbeat(&self) {
        *locked(&self.tick_started) = Instant::now();
    }

    pub(crate) fn attach_task(&self, task_id: &str) {
        *locked(&self.task) = Some((task_id.to_owned(), Instant::now()));
    }

    pub(crate) fn task_heartbeat(&self, task_id: &str) {
        if let Some((active, heartbeat)) = locked(&self.task).as_mut() {
            if active == task_id {
                *heartbeat = Instant::now();
            }
        }
    }

    pub(crate) fn detach_task(&self, task_id: &str) {
        let mut task = locked(&self.task);
        if task.as_ref().is_some_and(|(active, _)| active == task_id) {
            *task = None;
        }
    }

    pub(crate) fn heartbeat_age(&self) -> Duration {
        locked(&self.last_pulse).elapsed()
    }

    pub(crate) fn is_stalled(&self, task_grace: Duration, tick_grace: Duration) -> bool {
        match locked(&self.task).as_ref() {
            Some((_, heartbeat)) => heartbeat.elapsed() > task_grace,
            None => locked(&self.tick_started).elapsed() > tick_grace,
        }
    }
}

#[derive(Default)]
pub(crate) struct WorkerSupervision {
    pub(crate) task_claims: Mutex<HashMap<String, i64>>,
    configured: AtomicUsize,
    workers: Mutex<Vec<Arc<WorkerMonitor>>>,
    recovery: Mutex<Option<Arc<WorkerMonitor>>>,
    pub(crate) recovery_in_flight: AtomicBool,
    last_recovery_success: Mutex<Option<Instant>>,
    recovery_last_error: Mutex<Option<String>>,
}

impl WorkerSupervision {
    pub(crate) fn configure(&self, workers: Vec<Arc<WorkerMonitor>>, recovery: Arc<WorkerMonitor>) {
        self.configured.store(workers.len(), Ordering::Release);
        *locked(&self.workers) = workers;
        *locked(&self.recovery) = Some(recovery);
    }

    pub(crate) fn recovery_finished(&self, success: bool) {
        if success {
            *locked(&self.last_recovery_success) = Some(Instant::now());
            *locked(&self.recovery_last_error) = None;
        } else {
            *locked(&self.recovery_last_error) = Some("stale_task_recovery_failed".into());
        }
    }

    pub(crate) fn snapshot(
        &self,
        pulse_grace: Duration,
        recovery_grace: Duration,
    ) -> WorkerSupervisionHealth {
        let workers = locked(&self.workers);
        let configured = self.configured.load(Ordering::Acquire);
        let live = |monitor: &WorkerMonitor| {
            monitor.alive.load(Ordering::Acquire)
                && !monitor.stalled.load(Ordering::Acquire)
                && monitor.heartbeat_age() <= pulse_grace
        };
        let live_workers = workers.iter().filter(|worker| live(worker)).count();
        let stalled_workers = workers
            .iter()
            .filter(|worker| {
                worker.stalled.load(Ordering::Acquire)
                    || (worker.alive.load(Ordering::Acquire)
                        && worker.heartbeat_age() > pulse_grace)
            })
            .count();
        let recovery = locked(&self.recovery);
        let recovery_alive = recovery.as_ref().is_some_and(|worker| live(worker));
        let success_age = locked(&self.last_recovery_success)
            .as_ref()
            .map(Instant::elapsed);
        let last_error = locked(&self.recovery_last_error).clone();
        let state = if configured == 0 {
            "starting"
        } else if stalled_workers > 0
            || recovery
                .as_ref()
                .is_some_and(|worker| worker.stalled.load(Ordering::Acquire))
        {
            "stalled"
        } else if live_workers < configured
            || !recovery_alive
            || last_error.is_some()
            || success_age.is_none_or(|age| age > recovery_grace)
        {
            "degraded"
        } else {
            "running"
        };
        WorkerSupervisionHealth {
            state: state.into(),
            configured_workers: configured,
            live_workers,
            stalled_workers,
            restart_count: workers
                .iter()
                .map(|worker| worker.restarts.load(Ordering::Acquire))
                .sum::<u64>()
                + recovery
                    .as_ref()
                    .map_or(0, |worker| worker.restarts.load(Ordering::Acquire)),
            last_worker_heartbeat_age_seconds: workers
                .iter()
                .map(|worker| worker.heartbeat_age().as_secs())
                .max(),
            recovery_alive,
            last_recovery_heartbeat_age_seconds: recovery
                .as_ref()
                .map(|worker| worker.heartbeat_age().as_secs()),
            last_recovery_success_age_seconds: success_age.map(|age| age.as_secs()),
            recovery_last_error_code: last_error,
        }
    }
}

#[cfg(test)]
#[path = "worker_health_tests.rs"]
mod tests;

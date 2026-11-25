use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::runtime::worker_health::WorkerMonitor;
use crate::AppState;

tokio::task_local! {
    static CURRENT_WORKER: Arc<WorkerMonitor>;
}

pub(super) fn current_monitor() -> Option<Arc<WorkerMonitor>> {
    CURRENT_WORKER.try_with(Arc::clone).ok()
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct TickErrorDetail {
    pub(super) error_code: &'static str,
    pub(super) sqlite_extended_code: Option<i32>,
}

pub(super) fn tick_error_detail(error: &anyhow::Error) -> TickErrorDetail {
    for cause in error.chain() {
        if let Some(rusqlite::Error::SqliteFailure(sqlite, _)) =
            cause.downcast_ref::<rusqlite::Error>()
        {
            return TickErrorDetail {
                error_code: match sqlite.code {
                    rusqlite::ErrorCode::DatabaseBusy => "background_database_busy",
                    rusqlite::ErrorCode::DatabaseLocked => "background_database_locked",
                    _ => "background_sqlite_failed",
                },
                sqlite_extended_code: Some(sqlite.extended_code),
            };
        }
    }
    TickErrorDetail {
        error_code: "background_tick_failed",
        sqlite_extended_code: None,
    }
}

struct AbortOnDrop(JoinHandle<anyhow::Result<()>>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct MonitorLifetime(Arc<WorkerMonitor>);
impl Drop for MonitorLifetime {
    fn drop(&mut self) {
        self.0.alive.store(false, Ordering::Release);
    }
}

/// Keep ownership of every execution future. A panic cannot silently remove a worker loop.
/// A stale task heartbeat cancels that future, not a replay of its uncertain side effects.
pub(super) fn spawn_managed_loop<F, Fut>(
    monitor: Arc<WorkerMonitor>,
    pulse_interval: Duration,
    task_grace: Duration,
    tick_grace: Duration,
    poll_interval: Duration,
    work: F,
) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        let _lifetime = MonitorLifetime(monitor.clone());
        let work = Arc::new(work);
        let mut failures = 0_u32;
        loop {
            monitor.begin_tick();
            let tick_started_at = std::time::Instant::now();
            let work = work.clone();
            let mut child = AbortOnDrop(tokio::spawn(
                CURRENT_WORKER.scope(monitor.clone(), async move { work().await }),
            ));
            let mut pulse = tokio::time::interval(pulse_interval);
            pulse.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut abort_requested = false;
            let result = loop {
                tokio::select! {
                    result = &mut child.0 => break result,
                    _ = pulse.tick() => {
                        monitor.pulse();
                        if !abort_requested && monitor.is_stalled(task_grace, tick_grace) {
                            monitor.stalled.store(true, Ordering::Release);
                            child.0.abort();
                            abort_requested = true;
                            tracing::warn!(event = "worker_tick_stalled", worker = %monitor.name, "worker tick cancellation requested after lost progress");
                        }
                    }
                }
            };
            // An aborted waiter cannot cancel an already-started blocking DB tick.
            // Keep the old lease alive until its operation exits; never overlap a replacement.
            while monitor.blocking_operations.load(Ordering::Acquire) > 0 {
                monitor.pulse();
                tokio::time::sleep(pulse_interval).await;
            }
            match result {
                Ok(Ok(())) => {
                    failures = 0;
                    *monitor
                        .last_error_code
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                }
                Ok(Err(error)) => {
                    failures = failures.saturating_add(1);
                    let detail = tick_error_detail(&error);
                    *monitor
                        .last_error_code
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(detail.error_code.into());
                    monitor.alive.store(false, Ordering::Release);
                    tracing::error!(
                        event = "worker_tick_failed",
                        worker = %monitor.name,
                        elapsed_ms = tick_started_at.elapsed().as_millis(),
                        error_code = detail.error_code,
                        sqlite_extended_code = detail.sqlite_extended_code,
                        %error,
                        "supervised worker tick failed"
                    );
                }
                Err(error) => {
                    failures = failures.saturating_add(1);
                    monitor.restarts.fetch_add(1, Ordering::AcqRel);
                    *monitor
                        .last_error_code
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(
                        if error.is_panic() {
                            "background_tick_panicked"
                        } else {
                            "background_tick_cancelled"
                        }
                        .into(),
                    );
                    monitor.alive.store(false, Ordering::Release);
                    tracing::warn!(
                        event = "worker_tick_restarting",
                        worker = %monitor.name,
                        panicked = error.is_panic(),
                        cancelled = error.is_cancelled(),
                        "restarting supervised worker after execution future ended"
                    );
                }
            }
            let delay = if failures == 0 {
                poll_interval
            } else {
                poll_interval
                    .max(Duration::from_millis(250))
                    .saturating_mul(1_u32 << failures.min(6))
                    .min(Duration::from_secs(30))
            };
            let sleep = tokio::time::sleep(delay);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    _ = pulse.tick(), if failures == 0 => monitor.pulse(),
                }
            }
        }
    })
}

pub(crate) fn background_execution_heartbeat() {
    if let Some(monitor) = current_monitor() {
        monitor.execution_heartbeat();
    }
}

pub(super) struct BlockingOperation(Option<Arc<WorkerMonitor>>);
impl BlockingOperation {
    pub(super) fn start() -> Self {
        let monitor = current_monitor();
        if let Some(monitor) = &monitor {
            monitor.blocking_operations.fetch_add(1, Ordering::AcqRel);
        }
        Self(monitor)
    }
}
impl Drop for BlockingOperation {
    fn drop(&mut self) {
        if let Some(monitor) = &self.0 {
            monitor.blocking_operations.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

pub(crate) fn spawn_background_loop<F, Fut>(
    state: &AppState,
    name: String,
    idle: Duration,
    work: F,
) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<bool>> + Send + 'static,
{
    let monitor = state.worker.supervision.register_background(name);
    spawn_managed_loop(
        monitor,
        Duration::from_secs(5),
        Duration::from_secs(300),
        Duration::from_secs(600),
        Duration::from_millis(5),
        move || {
            let future = work();
            async move {
                if !future.await? {
                    tokio::time::sleep(idle).await;
                }
                Ok(())
            }
        },
    )
}

pub(crate) struct ActiveTaskGuard {
    state: AppState,
    task_id: String,
    claim_attempt: i64,
    cancellation: CancellationToken,
    heartbeat_stop: Option<oneshot::Sender<()>>,
    monitor: Option<Arc<WorkerMonitor>>,
    finished: bool,
}

impl ActiveTaskGuard {
    pub(crate) fn start(state: &AppState, task_id: &str, claim_attempt: i64) -> Self {
        let mut claims = state
            .worker
            .supervision
            .task_claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cancellation = state.worker.register_active_task(task_id);
        claims.insert(task_id.to_owned(), claim_attempt);
        let monitor = current_monitor();
        if let Some(monitor) = &monitor {
            monitor.attach_task(task_id);
        }
        drop(claims);
        Self {
            state: state.clone(),
            task_id: task_id.to_owned(),
            claim_attempt,
            cancellation,
            heartbeat_stop: Some(super::background_workers::start_task_heartbeat(
                state.clone(),
                task_id.to_owned(),
                claim_attempt,
            )),
            monitor,
            finished: false,
        }
    }

    pub(crate) fn finish(mut self) {
        self.finished = true;
        self.cleanup();
    }

    fn cleanup(&mut self) {
        if let Some(stop) = self.heartbeat_stop.take() {
            let _ = stop.send(());
        }
        let mut claims = self
            .state
            .worker
            .supervision
            .task_claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if claims.get(&self.task_id) == Some(&self.claim_attempt) {
            claims.remove(&self.task_id);
            self.state.worker.unregister_active_task(&self.task_id);
        }
        if let Some(monitor) = &self.monitor {
            monitor.detach_task(&self.task_id);
        }
    }
}

impl Drop for ActiveTaskGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.cancellation.cancel();
            self.cleanup();
        }
    }
}

#[cfg(test)]
#[path = "worker_supervision_tests.rs"]
mod tests;

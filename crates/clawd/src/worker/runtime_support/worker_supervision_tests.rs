use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};

async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("supervisor made progress");
}

fn start<F, Fut>(monitor: Arc<WorkerMonitor>, work: F) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    spawn_managed_loop(
        monitor,
        Duration::from_millis(5),
        Duration::from_millis(80),
        Duration::from_millis(160),
        Duration::from_millis(5),
        work,
    )
}

struct ActiveFuture(Arc<AtomicUsize>);
impl Drop for ActiveFuture {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[test]
fn slow_resume_renewals_are_inside_the_watchdog_grace() {
    let mut state = AppState::test_default_with_fixture_provider();
    state.worker.worker_task_heartbeat_seconds = 120;
    state.worker.worker_running_no_progress_timeout_seconds = 60;
    let (task, tick) = super::super::background_workers::execution_watchdog_graces(&state);
    assert_eq!(task, Duration::from_secs(480));
    assert_eq!(tick, task);
}

#[tokio::test]
async fn panic_restarts_worker_instead_of_silently_losing_it() {
    let monitor = WorkerMonitor::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let handle = start(monitor.clone(), move || {
        let count = count.clone();
        async move {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("injected_worker_panic");
            }
            Ok(())
        }
    });
    wait_for(|| calls.load(Ordering::SeqCst) >= 2).await;
    assert_eq!(monitor.restarts.load(Ordering::Acquire), 1);
    assert!(monitor.alive.load(Ordering::Acquire));
    handle.abort();
    let _ = handle.await;
    assert!(!monitor.alive.load(Ordering::Acquire));
}

#[tokio::test]
async fn lost_heartbeat_cancels_before_starting_any_replacement() {
    let monitor = WorkerMonitor::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let (count, current, max) = (calls.clone(), active.clone(), maximum.clone());
    let handle = start(monitor.clone(), move || {
        let (count, current, max) = (count.clone(), current.clone(), max.clone());
        async move {
            max.fetch_max(current.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            let _active = ActiveFuture(current);
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                current_monitor().unwrap().attach_task("lost-heartbeat");
                std::future::pending::<()>().await;
            }
            Ok(())
        }
    });
    wait_for(|| calls.load(Ordering::SeqCst) >= 2).await;
    assert!(monitor.restarts.load(Ordering::Acquire) >= 1);
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
    handle.abort();
    let _ = handle.await;
    wait_for(|| active.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test]
async fn healthy_long_task_is_not_subject_to_tick_wall_clock_limit() {
    let monitor = WorkerMonitor::new();
    let completed = Arc::new(AtomicBool::new(false));
    let done = completed.clone();
    let handle = start(monitor.clone(), move || {
        let done = done.clone();
        async move {
            let monitor = current_monitor().unwrap();
            monitor.attach_task("healthy-long-task");
            for _ in 0..50 {
                tokio::time::sleep(Duration::from_millis(6)).await;
                monitor.task_heartbeat("healthy-long-task");
            }
            monitor.detach_task("healthy-long-task");
            done.store(true, Ordering::Release);
            Ok(())
        }
    });
    wait_for(|| completed.load(Ordering::Acquire)).await;
    assert_eq!(monitor.restarts.load(Ordering::Acquire), 0);
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn idle_recovery_poll_keeps_its_supervisor_heartbeat_fresh() {
    let monitor = WorkerMonitor::new();
    let handle = spawn_managed_loop(
        monitor.clone(),
        Duration::from_millis(5),
        Duration::from_secs(1),
        Duration::from_secs(1),
        Duration::from_secs(30),
        || async { Ok(()) },
    );
    wait_for(|| monitor.alive.load(Ordering::Acquire)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(monitor.heartbeat_age() < Duration::from_millis(80));
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn aborted_execution_cleans_active_registration_and_its_own_token() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task_state = state.clone();
    let handle = tokio::spawn(async move {
        let _guard = ActiveTaskGuard::start(&task_state, "aborted-task", 1);
        std::future::pending::<()>().await;
    });
    wait_for(|| state.worker.task_runtime_is_registered("aborted-task")).await;
    let token = state
        .worker
        .task_cancellation_token("aborted-task")
        .unwrap();
    handle.abort();
    let _ = handle.await;
    assert!(token.is_cancelled());
    assert!(!state.worker.task_runtime_is_registered("aborted-task"));
    assert!(state
        .worker
        .task_cancellation_token("aborted-task")
        .is_none());
    assert!(state
        .worker
        .supervision
        .task_claims
        .lock()
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn dropping_old_claim_cannot_unregister_or_cancel_new_claim() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let old = ActiveTaskGuard::start(&state, "reclaimed-task", 1);
    let old_token = state
        .worker
        .task_cancellation_token("reclaimed-task")
        .unwrap();
    let new = ActiveTaskGuard::start(&state, "reclaimed-task", 2);
    let new_token = state
        .worker
        .task_cancellation_token("reclaimed-task")
        .unwrap();
    drop(old);
    assert!(old_token.is_cancelled());
    assert!(!new_token.is_cancelled());
    assert!(state.worker.task_runtime_is_registered("reclaimed-task"));
    new.finish();
    assert!(!new_token.is_cancelled());
    assert!(!state.worker.task_runtime_is_registered("reclaimed-task"));
}

#[tokio::test]
async fn independent_sweeper_recovers_orphan_without_ordinary_worker_tick() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_ask_task_row("orphan-task", 7, 11, r#"{"text":"fixture"}"#);
    let task = crate::repo::claim_next_task(&state).unwrap().unwrap();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET updated_at='1',lease_expires_at=1 WHERE task_id=?1",
            [&task.task_id],
        )
        .unwrap();
    super::super::recovery_flow::maybe_recover_stale_running_tasks_only(&state)
        .await
        .unwrap();
    let status: String = state
        .core
        .db
        .get()
        .unwrap()
        .query_row(
            "SELECT status FROM tasks WHERE task_id=?1",
            [&task.task_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "timeout");
    assert!(!state
        .worker
        .supervision
        .recovery_in_flight
        .load(Ordering::Acquire));
    assert!(state
        .worker
        .supervision
        .snapshot(Duration::from_secs(1), Duration::from_secs(1))
        .last_recovery_success_age_seconds
        .is_some());
}

#[tokio::test]
async fn cancelled_recovery_waiter_keeps_the_started_sweep_exclusive() {
    let mut state = AppState::test_default_with_fixture_provider();
    state.core.db = r2d2::Pool::builder()
        .max_size(1)
        .build(r2d2_sqlite::SqliteConnectionManager::memory())
        .unwrap();
    let state = state.with_seeded_db_schema();
    let held = state.core.db.get().unwrap();
    let sweep_state = state.clone();
    let sweep = tokio::spawn(async move {
        super::super::recovery_flow::maybe_recover_stale_running_tasks_only(&sweep_state).await
    });
    wait_for(|| {
        state
            .worker
            .supervision
            .recovery_in_flight
            .load(Ordering::Acquire)
    })
    .await;
    sweep.abort();
    let _ = sweep.await;
    *state.worker.last_running_recovery_check_ts.lock().unwrap() = 0;
    super::super::recovery_flow::maybe_recover_stale_running_tasks_only(&state)
        .await
        .unwrap();
    assert_eq!(
        *state.worker.last_running_recovery_check_ts.lock().unwrap(),
        0
    );
    assert!(state
        .worker
        .supervision
        .recovery_in_flight
        .load(Ordering::Acquire));
    drop(held);
    wait_for(|| {
        !state
            .worker
            .supervision
            .recovery_in_flight
            .load(Ordering::Acquire)
    })
    .await;
}

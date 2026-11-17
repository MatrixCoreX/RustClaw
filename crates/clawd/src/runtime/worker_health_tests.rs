use super::*;

fn snapshot(supervision: &WorkerSupervision) -> WorkerSupervisionHealth {
    supervision.snapshot(Duration::from_secs(15), Duration::from_secs(30))
}

#[test]
fn health_requires_live_workers_and_successful_independent_recovery() {
    let supervision = WorkerSupervision::default();
    assert_eq!(snapshot(&supervision).state, "starting");
    let worker = WorkerMonitor::new();
    let recovery = WorkerMonitor::new();
    supervision.configure(vec![worker.clone()], recovery.clone());
    assert_eq!(snapshot(&supervision).state, "degraded");
    worker.pulse();
    recovery.pulse();
    assert_eq!(snapshot(&supervision).state, "degraded");
    supervision.recovery_finished(true);
    let health = snapshot(&supervision);
    assert_eq!(health.state, "running");
    assert_eq!(health.live_workers, 1);
    assert!(health.recovery_alive);
    supervision.recovery_finished(false);
    let health = snapshot(&supervision);
    assert_eq!(health.state, "degraded");
    assert_eq!(
        health.recovery_last_error_code.as_deref(),
        Some("stale_task_recovery_failed")
    );
    supervision.recovery_finished(true);
    assert_eq!(snapshot(&supervision).state, "running");
}

#[test]
fn stale_heartbeat_and_dead_worker_cannot_report_running() {
    let supervision = WorkerSupervision::default();
    let worker = WorkerMonitor::new();
    let recovery = WorkerMonitor::new();
    supervision.configure(vec![worker.clone()], recovery.clone());
    worker.pulse();
    recovery.pulse();
    supervision.recovery_finished(true);
    *locked(&worker.last_pulse) = Instant::now() - Duration::from_secs(20);
    let health = snapshot(&supervision);
    assert_eq!(health.state, "stalled");
    assert_eq!(health.stalled_workers, 1);
    worker.alive.store(false, Ordering::Release);
    assert_eq!(snapshot(&supervision).state, "degraded");
    worker.pulse();
    *locked(&supervision.last_recovery_success) = Some(Instant::now() - Duration::from_secs(31));
    assert_eq!(snapshot(&supervision).state, "degraded");
}

#[test]
fn task_and_resume_heartbeats_protect_long_executions() {
    let worker = WorkerMonitor::new();
    *locked(&worker.tick_started) = Instant::now() - Duration::from_secs(100);
    assert!(worker.is_stalled(Duration::from_secs(10), Duration::from_secs(10)));
    worker.attach_task("owned-task");
    assert!(!worker.is_stalled(Duration::from_secs(10), Duration::from_secs(10)));
    locked(&worker.task).as_mut().unwrap().1 = Instant::now() - Duration::from_secs(100);
    worker.task_heartbeat("different-task");
    assert!(worker.is_stalled(Duration::from_secs(10), Duration::from_secs(10)));
    worker.task_heartbeat("owned-task");
    assert!(!worker.is_stalled(Duration::from_secs(10), Duration::from_secs(10)));
    worker.detach_task("owned-task");
    worker.execution_heartbeat();
    assert!(!worker.is_stalled(Duration::from_secs(10), Duration::from_secs(10)));
}

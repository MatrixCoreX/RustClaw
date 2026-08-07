use super::recovery_flow::resume_execution_lease_seconds;

#[test]
fn planner_resume_lease_has_a_five_minute_floor() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.worker.worker_task_heartbeat_seconds = 10;

    assert_eq!(resume_execution_lease_seconds(&state), 300);
}

#[test]
fn planner_resume_lease_scales_with_slower_heartbeats() {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.worker.worker_task_heartbeat_seconds = 120;

    assert_eq!(resume_execution_lease_seconds(&state), 480);
}

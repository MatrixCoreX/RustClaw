use super::*;
use crate::worker::runtime_support::{
    run_with_renewable_resume_execution_lease, RenewableResumeExecution,
};

fn claimed_fixture() -> (
    crate::AppState,
    repo::ClaimedDispatchedPausedCheckpointResumeExecution,
) {
    let state = test_state();
    state.seed_ask_task_row("resume-lease", 7, 11, r#"{"text":"fixture"}"#);
    let task = repo::claim_next_task(&state).unwrap().unwrap();
    let now = crate::now_ts_u64() as i64;
    let expires_at = now + 60;
    let checkpoint_id = "lease-checkpoint";
    let executor_state = "executing_planner_resume";
    let executor_action = "run_seeded_agent_loop";
    let executor_status = "seeded_loop_requires_provider_window";
    let dispatch_state = "ready_to_run_seeded_agent_loop";
    let base = json!({
        "checkpoint_id":checkpoint_id, "executor_state":executor_state,
        "executor_action":executor_action, "executor_status":executor_status,
        "dispatch_state":dispatch_state, "expires_at":expires_at
    });
    let claim = |owner: &str| {
        let mut value = base.clone();
        value["owner"] = json!(owner);
        value
    };
    let mut result = checkpoint_result(checkpoint_id, now);
    result["task_lifecycle"] = json!({
        "state":"running", "checkpoint_id":checkpoint_id,
        "resume_claim":claim(&state.worker.worker_id),
        "resume_executor_claim":claim("worker_recovery_executor"),
        "resume_executor_handoff_claim":claim("worker_recovery_handoff_executor"),
        "resume_executor_dispatch_claim":claim("worker_recovery_dispatch_executor"),
        "resume_executor":base
    });
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE tasks SET result_json=?1, lease_expires_at=?2 WHERE task_id=?3",
            rusqlite::params![result.to_string(), expires_at, task.task_id],
        )
        .unwrap();
    let claimed = repo::ClaimedDispatchedPausedCheckpointResumeExecution {
        task_id: task.task_id.clone(),
        task,
        checkpoint_id: checkpoint_id.into(),
        executor_state: executor_state.into(),
        executor_action: executor_action.into(),
        executor_status: executor_status.into(),
        dispatch_state: dispatch_state.into(),
        dispatch_execution_state: "claimed_to_run_seeded_agent_loop".into(),
        resume_trigger: "worker_recovery".into(),
        resume_directive: "run_next_planner_round".into(),
        lease_expires_at: expires_at,
        handoff_claim_expires_at: expires_at,
        dispatch_claim_expires_at: expires_at,
        execution_plan: json!({}),
        dispatch_payload: json!({}),
        dispatch_claim: json!({}),
        task_checkpoint: crate::task_lifecycle::task_checkpoint_from_result_json(&result).unwrap(),
    };
    (state, claimed)
}

#[tokio::test(flavor = "current_thread")]
async fn resume_lease_pool_wait_keeps_async_runtime_responsive() {
    let (state, claimed) = claimed_fixture();
    let held = state.core.db.get().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });
    let started = Instant::now();
    let (outcome, timer_delay) = tokio::join!(
        run_with_renewable_resume_execution_lease(&state, &claimed, 30, async { 42 }),
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        }
    );
    release.join().unwrap();
    assert!(matches!(
        outcome.unwrap(),
        RenewableResumeExecution::Completed(42)
    ));
    eprintln!(
        "stage=resume_lease pool_hold_ms=500 timer_delay_ms={}",
        timer_delay.as_millis()
    );
    assert!(timer_delay < Duration::from_millis(400));
}

#[tokio::test]
async fn running_execution_renews_its_complete_claim_chain_more_than_once() {
    let (state, claimed) = claimed_fixture();
    let outcome = run_with_renewable_resume_execution_lease(&state, &claimed, 3, async {
        tokio::time::sleep(Duration::from_millis(2200)).await;
        42
    })
    .await
    .unwrap();
    assert!(matches!(outcome, RenewableResumeExecution::Completed(42)));
    let result = task_result(&state, &claimed.task_id);
    for field in [
        "resume_claim",
        "resume_executor_claim",
        "resume_executor_handoff_claim",
        "resume_executor_dispatch_claim",
    ] {
        assert!(
            result["task_lifecycle"][field]["renewal_count"]
                .as_u64()
                .unwrap()
                >= 2,
            "{field}"
        );
        assert!(
            result["task_lifecycle"][field]["expires_at"]
                .as_i64()
                .unwrap()
                > crate::now_ts_u64() as i64
        );
    }
}

#[tokio::test]
async fn cancellation_during_execution_loses_the_lease_without_returning_completion() {
    let (state, claimed) = claimed_fixture();
    let mutation_state = state.clone();
    let cancel = async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        mutation_state
            .core
            .db
            .get()
            .unwrap()
            .execute("UPDATE tasks SET status='canceled'", [])
            .unwrap();
    };
    let (outcome, ()) = tokio::join!(
        run_with_renewable_resume_execution_lease(
            &state,
            &claimed,
            3,
            std::future::pending::<()>()
        ),
        cancel,
    );
    assert!(matches!(
        outcome.unwrap(),
        RenewableResumeExecution::LeaseLost
    ));
    assert_eq!(
        state
            .core
            .db
            .get()
            .unwrap()
            .query_row("SELECT status FROM tasks", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "canceled"
    );
}

#[tokio::test]
async fn expired_claim_does_not_poll_the_execution_future() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (state, claimed) = claimed_fixture();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("UPDATE tasks SET lease_expires_at=0", [])
        .unwrap();
    let polled = AtomicBool::new(false);
    let outcome = run_with_renewable_resume_execution_lease(&state, &claimed, 3, async {
        polled.store(true, Ordering::SeqCst);
    })
    .await
    .unwrap();
    assert!(matches!(outcome, RenewableResumeExecution::LeaseLost));
    assert!(!polled.load(Ordering::SeqCst));
}

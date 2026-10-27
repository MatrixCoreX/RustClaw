use super::*;

fn ready_result(checkpoint: &str, now: i64) -> serde_json::Value {
    json!({
        "task_lifecycle": {
            "state": "background", "checkpoint_id": checkpoint, "next_check_after": now,
            "resume_executor": {
                "checkpoint_id": checkpoint, "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery", "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json(checkpoint, vec![])
    })
}

#[test]
fn bounded_resume_scan_eventually_reaches_valid_work_after_invalid_prefix() {
    let state = state_with_tasks_table();
    let now = 10_000;
    let mut invalid = ready_result("invalid", now);
    invalid["task_checkpoint"]["checkpoint_id"] = json!("mismatch");
    for index in 0..160 {
        insert_task(
            &state,
            &format!("invalid-{index:03}"),
            "running",
            Some(&invalid),
            1,
        );
    }
    insert_task(
        &state,
        "valid",
        "running",
        Some(&ready_result("valid", now)),
        2,
    );
    let mut found = false;
    for _ in 0..20 {
        let ready = list_ready_paused_checkpoint_resume_executors_internal(&state, now, 1).unwrap();
        assert!(ready.len() <= 1);
        assert!(ready.iter().all(|item| item.task_id == "valid"));
        found |= ready.iter().any(|item| item.task_id == "valid");
    }
    assert!(
        found,
        "a bounded scan must not restart forever at the invalid prefix"
    );
}

fn add_rejected_prefix(state: &crate::AppState, count: usize, now: i64) {
    for index in 0..count {
        let mut invalid = ready_result("invalid", now);
        match index % 3 {
            0 => invalid["task_checkpoint"]["checkpoint_id"] = json!("mismatch"),
            1 => invalid["task_lifecycle"]["resume_executor"]["resume_directive"] = json!(""),
            _ => {
                invalid["task_lifecycle"]["resume_work_item"] = json!({"checkpoint_id":"mismatch"})
            }
        }
        insert_task(
            state,
            &format!("rejected-{index:03}"),
            "running",
            Some(&invalid),
            1,
        );
    }
}

fn insert_owned_ready(state: &crate::AppState, id: &str, now: i64) {
    let mut ready = ready_result(id, now);
    ready["task_lifecycle"]["resume_claim"] = json!({
        "checkpoint_id":id, "owner":state.worker.worker_id, "expires_at":now + 300
    });
    insert_task(state, id, "running", Some(&ready), 2);
    set_task_lease(state, id, &state.worker.worker_id, now + 300, 1, now);
}

#[test]
fn mixed_resume_queue_drains_once_without_resuming_future_leased_or_cancelled_work() {
    let state = state_with_tasks_table();
    let now = 10_000;
    add_rejected_prefix(&state, 320, now);
    for index in 0..100 {
        let future = ready_result("future", now + 600);
        insert_task(
            &state,
            &format!("future-{index}"),
            "running",
            Some(&future),
            1,
        );
        let mut leased = ready_result("leased", now);
        leased["task_lifecycle"]["resume_executor"]["executor_state"] =
            json!("executing_planner_resume");
        leased["task_lifecycle"]["resume_executor_claim"] =
            json!({"checkpoint_id":"leased", "expires_at":now + 600});
        insert_task(
            &state,
            &format!("leased-{index}"),
            "running",
            Some(&leased),
            1,
        );
        insert_task(
            &state,
            &format!("cancelled-{index}"),
            "canceled",
            Some(&ready_result("cancelled", now)),
            1,
        );
    }
    for index in 0..24 {
        insert_owned_ready(&state, &format!("valid-{index:02}"), now);
    }
    let mut claimed_ids = std::collections::HashSet::new();
    for _ in 0..200 {
        for ready in list_ready_paused_checkpoint_resume_executors_internal(&state, now, 3).unwrap()
        {
            assert!(ready.task_id.starts_with("valid-"));
            let claim = claim_ready_paused_checkpoint_resume_executor_internal(
                &state,
                &ready.task_id,
                &ready.checkpoint_id,
                &ready.executor_state,
                now,
                300,
            )
            .unwrap();
            assert!(claim.is_some());
            assert!(claimed_ids.insert(ready.task_id));
        }
        if claimed_ids.len() == 24 {
            break;
        }
    }
    assert_eq!(claimed_ids.len(), 24);
}

#[test]
fn resume_scan_wraps_after_queue_shrink_and_process_restart() {
    let mut state = state_with_tasks_table();
    let now = 10_000;
    add_rejected_prefix(&state, 160, now);
    assert!(
        list_ready_paused_checkpoint_resume_executors_internal(&state, now, 1)
            .unwrap()
            .is_empty()
    );
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("DELETE FROM tasks", [])
        .unwrap();
    insert_owned_ready(&state, "new-work", now);
    assert!((0..3).any(
        |_| list_ready_paused_checkpoint_resume_executors_internal(&state, now, 1)
            .unwrap()
            .iter()
            .any(|ready| ready.task_id == "new-work")
    ));
    state.worker = crate::WorkerConfig::test_default();
    assert_eq!(
        list_ready_paused_checkpoint_resume_executors_internal(&state, now, 1).unwrap()[0].task_id,
        "new-work"
    );
}

#[test]
fn concurrent_resume_scans_cannot_duplicate_durable_claims() {
    let state = state_with_tasks_table();
    let now = 10_000;
    add_rejected_prefix(&state, 64, now);
    for index in 0..12 {
        insert_owned_ready(&state, &format!("valid-{index:02}"), now);
    }
    let claims = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let threads = (0..4)
        .map(|_| {
            let (state, claims) = (state.clone(), claims.clone());
            std::thread::spawn(move || {
                for _ in 0..100 {
                    for ready in
                        list_ready_paused_checkpoint_resume_executors_internal(&state, now, 2)
                            .unwrap()
                    {
                        if claim_ready_paused_checkpoint_resume_executor_internal(
                            &state,
                            &ready.task_id,
                            &ready.checkpoint_id,
                            &ready.executor_state,
                            now,
                            300,
                        )
                        .unwrap()
                        .is_some()
                        {
                            assert!(claims.lock().unwrap().insert(ready.task_id));
                        }
                    }
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(claims.lock().unwrap().len(), 12);
}

#[test]
fn cancellation_between_scan_and_claim_keeps_the_task_cancelled() {
    let state = state_with_tasks_table();
    let now = 10_000;
    insert_owned_ready(&state, "cancel-after-scan", now);
    let ready = list_ready_paused_checkpoint_resume_executors_internal(&state, now, 1)
        .unwrap()
        .remove(0);
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("UPDATE tasks SET status='canceled'", [])
        .unwrap();
    assert!(claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        &ready.task_id,
        &ready.checkpoint_id,
        &ready.executor_state,
        now,
        300,
    )
    .unwrap()
    .is_none());
}

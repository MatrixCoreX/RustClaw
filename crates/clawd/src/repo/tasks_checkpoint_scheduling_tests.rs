use super::*;

#[test]
fn list_due_paused_checkpoint_tasks_filters_and_orders_machine_checkpoints() {
    let state = state_with_tasks_table();
    let now = 1_000;
    let due_from_journal = json!({
        "task_journal": {
            "summary": {
                "task_lifecycle": {
                    "state": "background",
                    "resume_reason": "async_job_poll",
                    "next_check_after": now,
                    "checkpoint_id": "ckpt-journal"
                },
                "task_checkpoint": checkpoint_json("ckpt-journal", vec!["external_call:job-1"])
            }
        }
    });
    let due_from_root = json!({
        "task_lifecycle": {
            "state": "waiting",
            "resume_reason": "agent_loop_soft_budget",
            "next_check_after": now - 10,
            "checkpoint_id": "ckpt-root"
        },
        "task_checkpoint": checkpoint_json("ckpt-root", vec![])
    });
    let future_wait = json!({
        "task_lifecycle": {
            "state": "waiting",
            "resume_reason": "provider_gap_retry_window",
            "next_check_after": now + 60,
            "checkpoint_id": "ckpt-future"
        },
        "task_checkpoint": checkpoint_json("ckpt-future", vec![])
    });
    let invalid_checkpoint = json!({
        "task_lifecycle": {
            "state": "waiting",
            "resume_reason": "agent_loop_soft_budget",
            "next_check_after": now
        }
    });
    let mut user_input_checkpoint = checkpoint_json("ckpt-user-input", vec![]);
    user_input_checkpoint["resume_entrypoint"] = json!("await_user_input");
    let awaiting_user = json!({
        "task_lifecycle": {
            "state": "needs_user",
            "resume_reason": "confirmation_required",
            "checkpoint_id": "ckpt-user-input"
        },
        "task_checkpoint": user_input_checkpoint
    });

    insert_task(
        &state,
        "due-journal",
        "running",
        Some(&due_from_journal),
        10,
    );
    insert_task(&state, "future-wait", "running", Some(&future_wait), 20);
    insert_task(&state, "invalid", "running", Some(&invalid_checkpoint), 30);
    insert_task(&state, "awaiting-user", "running", Some(&awaiting_user), 35);
    insert_task(&state, "due-root", "running", Some(&due_from_root), 40);
    insert_task(
        &state,
        "terminal-ignored",
        "succeeded",
        Some(&due_from_root),
        1,
    );

    let first =
        list_due_paused_checkpoint_tasks_internal(&state, now, 1).expect("list first due task");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].task_id, "due-journal");
    assert_eq!(first[0].lifecycle_state, "background");
    assert_eq!(first[0].checkpoint_id, "ckpt-journal");
    assert_eq!(first[0].task_checkpoint.checkpoint_id, "ckpt-journal");
    assert_eq!(
        first[0].task_checkpoint.completed_side_effect_refs,
        vec!["external_call:job-1"]
    );
    assert_eq!(first[0].resume_entrypoint, "next_planner_round");
    assert_eq!(first[0].resume_directive, "run_next_planner_round");
    assert_eq!(first[0].resume_wait_seconds, 0);
    assert_eq!(first[0].completed_side_effect_count, 1);
    assert!(first[0].requires_idempotency_guard);

    let all = list_due_paused_checkpoint_tasks_internal(&state, now, 10).expect("list due tasks");
    let task_ids: Vec<_> = all.iter().map(|task| task.task_id.as_str()).collect();
    assert_eq!(task_ids, vec!["due-journal", "due-root"]);
    assert_eq!(all[1].lifecycle_state, "waiting");
    assert_eq!(all[1].checkpoint_id, "ckpt-root");
    assert_eq!(all[1].completed_side_effect_count, 0);
    assert!(!all[1].requires_idempotency_guard);

    assert!(
        claim_due_paused_checkpoint_task_internal(
            &state,
            "awaiting-user",
            "ckpt-user-input",
            now,
            30,
        )
        .expect("reject automatic user-input checkpoint claim")
        .is_none(),
        "user-input checkpoints must only resume after an explicit user decision"
    );
    let db = state.core.db.get().expect("get db");
    let (lease_owner, lease_expires_at): (Option<String>, i64) = db
        .query_row(
            "SELECT lease_owner, lease_expires_at FROM tasks WHERE task_id = 'awaiting-user'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query awaiting-user lease");
    assert_eq!(lease_owner, None);
    assert_eq!(lease_expires_at, 0);
}

#[test]
fn claim_due_paused_checkpoint_task_sets_machine_resume_lease() {
    let state = state_with_tasks_table();
    let now = 2_000;
    let due = json!({
        "task_lifecycle": {
            "state": "waiting",
            "resume_reason": "agent_loop_soft_budget",
            "next_check_after": now - 1,
            "checkpoint_id": "ckpt-claim"
        },
        "task_checkpoint": checkpoint_json("ckpt-claim", vec!["write_file:tmp/report.txt"])
    });
    insert_task(&state, "claim-me", "running", Some(&due), 100);

    let wrong =
        claim_due_paused_checkpoint_task_internal(&state, "claim-me", "ckpt-other", now, 30)
            .expect("claim wrong checkpoint");
    assert!(wrong.is_none());

    let claimed =
        claim_due_paused_checkpoint_task_internal(&state, "claim-me", "ckpt-claim", now, 30)
            .expect("claim due checkpoint")
            .expect("claimed");
    assert_eq!(claimed.task_id, "claim-me");
    assert_eq!(claimed.task_checkpoint.checkpoint_id, "ckpt-claim");
    assert_eq!(
        claimed.task_checkpoint.completed_side_effect_refs,
        vec!["write_file:tmp/report.txt"]
    );
    assert_eq!(claimed.resume_entrypoint, "next_planner_round");
    assert_eq!(claimed.resume_directive, "run_next_planner_round");
    assert_eq!(claimed.completed_side_effect_count, 1);
    assert!(claimed.requires_idempotency_guard);

    let mismatched_work_item = json!({
        "schema_version": 1,
        "task_id": "claim-me",
        "checkpoint_id": "ckpt-other",
        "executor_state": "prepared"
    });
    assert!(
        !record_paused_checkpoint_resume_work_item_internal(
            &state,
            claimed.claim_attempt,
            "claim-me",
            "ckpt-claim",
            &mismatched_work_item,
            now + 1,
        )
        .expect("record mismatched work item"),
        "mismatched checkpoint work item must not be persisted"
    );

    let work_item = json!({
        "schema_version": 1,
        "task_id": "claim-me",
        "checkpoint_id": "ckpt-claim",
        "executor_state": "prepared",
        "resume_directive": "run_next_planner_round"
    });
    assert!(
        record_paused_checkpoint_resume_work_item_internal(
            &state,
            claimed.claim_attempt,
            "claim-me",
            "ckpt-claim",
            &work_item,
            now + 2,
        )
        .expect("record work item"),
        "matching checkpoint work item should be persisted"
    );

    let mismatched_executor = json!({
        "checkpoint_id": "ckpt-other",
        "resume_directive": "run_next_planner_round"
    });
    assert!(
        !record_paused_checkpoint_resume_executor_state_internal(
            &state,
            claimed.claim_attempt,
            "claim-me",
            "ckpt-claim",
            "ready_for_planner_resume",
            &mismatched_executor,
            Some("background"),
            Some(now + 2),
            now + 3,
        )
        .expect("record mismatched executor state"),
        "mismatched executor checkpoint must not be persisted"
    );

    let executor = json!({
        "checkpoint_id": "ckpt-claim",
        "resume_directive": "run_next_planner_round",
        "requires_idempotency_guard": true
    });
    assert!(
        record_paused_checkpoint_resume_executor_state_internal(
            &state,
            claimed.claim_attempt,
            "claim-me",
            "ckpt-claim",
            "ready_for_planner_resume",
            &executor,
            Some("background"),
            Some(now + 5),
            now + 4,
        )
        .expect("record executor state"),
        "matching executor checkpoint should be persisted"
    );

    let active =
        list_due_paused_checkpoint_tasks_internal(&state, now + 10, 10).expect("list during lease");
    assert!(
        active.is_empty(),
        "active lease should suppress duplicate resume candidates"
    );

    let after_expiry = list_due_paused_checkpoint_tasks_internal(&state, now + 31, 10)
        .expect("list after lease expiry");
    assert_eq!(after_expiry.len(), 1);
    assert_eq!(after_expiry[0].task_id, "claim-me");

    let task_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    let db = state.core.db.get().expect("get db");
    db.execute(
        "UPDATE tasks SET task_id = ?1 WHERE task_id = 'claim-me'",
        rusqlite::params![task_id.to_string()],
    )
    .expect("rename task for query api");
    drop(db);
    let (response, _, _) = get_task_query_record(&state, task_id)
        .expect("query claimed task")
        .expect("task exists");
    let lifecycle = response.lifecycle.expect("lifecycle");
    assert_eq!(lifecycle["resume_claim"]["checkpoint_id"], "ckpt-claim");
    assert_eq!(lifecycle["resume_claim"]["owner"], state.worker.worker_id);
    assert_eq!(lifecycle["resume_claim"]["owner_layer"], "worker_recovery");
    assert_eq!(
        lifecycle["resume_claim"]["executor_state"],
        "ready_for_planner_resume"
    );
    assert_eq!(lifecycle["resume_claim"]["prepared_at"], now + 2);
    assert_eq!(lifecycle["resume_claim"]["executor_state_at"], now + 4);
    assert_eq!(lifecycle["resume_work_item"]["checkpoint_id"], "ckpt-claim");
    assert_eq!(
        lifecycle["resume_work_item"]["executor_state"],
        "ready_for_planner_resume"
    );
    assert_eq!(
        lifecycle["resume_work_item"]["resume_directive"],
        "run_next_planner_round"
    );
    assert_eq!(
        lifecycle["resume_executor"]["executor_state"],
        "ready_for_planner_resume"
    );
    assert_eq!(
        lifecycle["resume_executor"]["resume_directive"],
        "run_next_planner_round"
    );
    assert_eq!(lifecycle["next_check_after"], now + 5);

    let db = state.core.db.get().expect("get db");
    let (lease_owner, lease_expires_at): (String, i64) = db
        .query_row(
            "SELECT lease_owner, lease_expires_at FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select task lease fields");
    assert_eq!(lease_owner, state.worker.worker_id);
    assert_eq!(lease_expires_at, now + 30);
}

#[test]
fn due_checkpoint_waits_for_frontend_worker_lease_and_claim_rechecks_it() {
    let state = state_with_tasks_table();
    let now = 5_000;
    let due = json!({
        "task_lifecycle": {
            "state": "waiting",
            "resume_reason": "task_budget_slice_exhausted",
            "next_check_after": now - 1,
            "checkpoint_id": "ckpt-worker-owned"
        },
        "task_checkpoint": checkpoint_json("ckpt-worker-owned", vec![])
    });
    insert_task(
        &state,
        "worker-owned-checkpoint",
        "running",
        Some(&due),
        now - 10,
    );
    set_task_lease(
        &state,
        "worker-owned-checkpoint",
        state.worker.worker_id.as_str(),
        now + 120,
        1,
        now - 10,
    );

    assert!(list_due_paused_checkpoint_tasks_internal(&state, now, 10)
        .expect("list while foreground lease is active")
        .is_empty());
    assert!(claim_due_paused_checkpoint_task_internal(
        &state,
        "worker-owned-checkpoint",
        "ckpt-worker-owned",
        now,
        30,
    )
    .expect("claim while foreground lease is active")
    .is_none());

    update_task_checkpointed_result(&state, "worker-owned-checkpoint", 1, &due.to_string())
        .expect("release foreground lease with checkpoint finalization");
    let due_after_release = list_due_paused_checkpoint_tasks_internal(&state, now, 10)
        .expect("list after foreground lease release");
    assert_eq!(due_after_release.len(), 1);
    assert_eq!(due_after_release[0].task_id, "worker-owned-checkpoint");
}

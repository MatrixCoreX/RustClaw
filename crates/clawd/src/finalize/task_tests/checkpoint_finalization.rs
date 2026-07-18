use serde_json::json;

fn state_with_tasks_table() -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("get db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            chat_id INTEGER NOT NULL,
            user_key TEXT,
            channel TEXT NOT NULL,
            external_user_id TEXT,
            external_chat_id TEXT,
            message_id INTEGER,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0,
            claim_attempt INTEGER NOT NULL DEFAULT 0,
            claimed_at INTEGER NOT NULL DEFAULT 0
        );",
    )
    .expect("create tasks table");
    drop(db);
    state
}

fn claimed_ask_task(task_id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({"text": "start long task"}).to_string(),
    }
}

fn insert_running_task(state: &crate::AppState, task: &crate::ClaimedTask) {
    let db = state.core.db.get().expect("get db");
    let now = crate::now_ts();
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', NULL, NULL, ?8, ?8)",
        rusqlite::params![
            task.task_id,
            task.user_id,
            task.chat_id,
            task.user_key,
            task.channel,
            task.kind,
            task.payload_json,
            now,
        ],
    )
    .expect("insert running task");
}

fn task_status_and_result(state: &crate::AppState, task_id: &str) -> (String, serde_json::Value) {
    let db = state.core.db.get().expect("get db");
    let (status, raw_result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select task result");
    (
        status,
        serde_json::from_str(&raw_result).expect("parse result_json"),
    )
}

#[tokio::test]
async fn checkpointed_ask_finalization_overrides_failure_metric() {
    let state = state_with_tasks_table();
    let task = claimed_ask_task("task-checkpoint-finalize");
    insert_running_task(&state, &task);
    {
        let db = state.core.db.get().expect("get db");
        db.execute(
            "UPDATE tasks
             SET lease_owner = 'worker:foreground',
                 lease_expires_at = 1781800300
             WHERE task_id = ?1",
            rusqlite::params![task.task_id],
        )
        .expect("set foreground lease");
    }

    let mut journal =
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "start long task");
    journal.record_task_lifecycle(json!({
        "schema_version": 1,
        "state": "waiting",
        "source": "agent_loop_soft_budget",
        "resume_reason": "agent_loop_max_rounds",
        "next_check_after": 1781800060,
        "checkpoint_id": "ckpt-accepted"
    }));
    journal.record_task_checkpoint(json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-accepted",
        "resume_entrypoint": "next_planner_round"
    }));
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);

    super::finalize_ask_checkpointed(
        &state,
        &task,
        r#"{"checkpoint_id":"ckpt-accepted","next_check_after":1781800060}"#,
        &[],
        None,
        &mut journal,
    )
    .await
    .expect("checkpoint finalize");

    let (status, result) = task_status_and_result(&state, &task.task_id);
    assert_eq!(status, "running");
    assert_eq!(result["task_journal"]["summary"]["final_status"], "success");
    assert_eq!(
        result["task_journal"]["summary"]["task_lifecycle"]["state"],
        "waiting"
    );
    assert_eq!(
        result["task_journal"]["summary"]["task_lifecycle"]["checkpoint_id"],
        "ckpt-accepted"
    );
    let db = state.core.db.get().expect("get db");
    let (lease_owner, lease_expires_at): (Option<String>, i64) = db
        .query_row(
            "SELECT lease_owner, lease_expires_at
             FROM tasks
             WHERE task_id = ?1",
            rusqlite::params![task.task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select released lease");
    assert!(lease_owner.is_none());
    assert_eq!(lease_expires_at, 0);
}

#[tokio::test]
async fn checkpointed_ask_finalization_preserves_pending_approval_context() {
    let state = state_with_tasks_table();
    let task = claimed_ask_task("task-approval-checkpoint-finalize");
    insert_running_task(&state, &task);
    let mut journal =
        crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "mutating task");
    journal.record_task_lifecycle(json!({
        "schema_version": 1,
        "state": "needs_user",
        "source": "plan_verifier",
        "resume_reason": "confirmation_required",
        "checkpoint_id": "ckpt-approval"
    }));
    journal.record_task_checkpoint(json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-approval",
        "boundary_context": {},
        "last_successful_round": null,
        "last_successful_step": null,
        "pending_action": null,
        "observations": [],
        "evidence_refs": [],
        "artifact_refs": [],
        "completed_side_effect_refs": [],
        "budget": {
            "round": 1,
            "step": 0,
            "llm_calls": 1,
            "tool_calls": 0,
            "elapsed_ms": 1,
            "llm_elapsed_ms": 1,
            "tool_elapsed_ms": 0
        },
        "resume_entrypoint": "await_user_input"
    }));
    let resume_context = json!({
        "required_decision": "require_confirmation",
        "approval_request": {
            "request_id": "approval-1",
            "status": "pending",
            "allowed_decisions": ["approve_once", "deny"]
        }
    });

    super::finalize_ask_checkpointed(&state, &task, "", &[], Some(&resume_context), &mut journal)
        .await
        .expect("approval checkpoint finalize");

    let (status, result) = task_status_and_result(&state, &task.task_id);
    assert_eq!(status, "running");
    assert_eq!(result["resume_context"], resume_context);
    assert_eq!(
        result["task_journal"]["summary"]["task_lifecycle"]["state"],
        "needs_user"
    );
    assert!(matches!(
        crate::task_lifecycle::checkpoint_resume_directive(&result, crate::now_ts_u64() as i64 + 1),
        crate::task_lifecycle::CheckpointResumeDirective::AwaitUserInput { .. }
    ));
}

#[tokio::test]
async fn provider_outage_finalizes_as_resumable_waiting_checkpoint() {
    let state = state_with_tasks_table();
    let task = claimed_ask_task("task-provider-wait-finalize");
    insert_running_task(&state, &task);
    state.note_task_llm_call_with_label_and_prompt_size(&task.task_id, "plan", 42);
    state.note_task_provider_blocker(
        &task.task_id,
        crate::TaskProviderBlocker {
            provider: "fixture-provider".to_string(),
            status_code: "rate_limited".to_string(),
            retry_after_seconds: 60,
            external_provider_blocked: true,
            message_key: "provider.rate_limited".to_string(),
        },
    );
    let payload = json!({"text": "start long task"});

    super::finalize_ask_result(
        &state,
        &task,
        &payload,
        "start long task",
        "",
        None,
        "start long task",
        None,
        &[],
        None,
        Err("opaque-provider-failure".to_string()),
    )
    .await
    .expect("provider wait finalization");

    let (status, result) = task_status_and_result(&state, &task.task_id);
    assert_eq!(status, "running");
    let lifecycle = &result["task_journal"]["summary"]["task_lifecycle"];
    assert_eq!(lifecycle["state"], "waiting");
    assert_eq!(
        lifecycle["resume_reason"],
        "provider_blocker_wait_background"
    );
    assert_eq!(lifecycle["provider_status"]["status_code"], "rate_limited");
    assert_eq!(lifecycle["provider_status"]["retry_after_seconds"], 60);
    assert_eq!(
        lifecycle["provider_status"]["message_key"],
        "provider.rate_limited"
    );
    let checkpoint = &result["task_journal"]["summary"]["task_checkpoint"];
    assert_eq!(checkpoint["resume_entrypoint"], "next_planner_round");
    assert_eq!(
        checkpoint["repair_signal"]["next_recovery_kind"],
        "wait_background"
    );
    let lifecycle_projection =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&result), None);
    assert_eq!(lifecycle_projection["provider_blocker_active"], true);
    assert_eq!(
        lifecycle_projection["provider_blocker_status_code"],
        "rate_limited"
    );
    let next_check_after = lifecycle["next_check_after"]
        .as_i64()
        .expect("next_check_after");
    assert!(matches!(
        crate::task_lifecycle::checkpoint_resume_directive(&result, next_check_after + 1),
        crate::task_lifecycle::CheckpointResumeDirective::RunNextPlannerRound { .. }
    ));
    assert!(result["text"].as_str().is_some_and(str::is_empty));
    assert!(state.task_provider_blocker(&task.task_id).is_none());
}

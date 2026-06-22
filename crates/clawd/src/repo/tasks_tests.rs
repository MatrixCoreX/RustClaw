use serde_json::json;
use uuid::Uuid;

use super::{
    claim_due_paused_checkpoint_task_internal,
    claim_ready_paused_checkpoint_resume_executor_internal, get_task_query_record,
    list_active_tasks_internal, list_due_paused_checkpoint_tasks_internal,
    list_ready_paused_checkpoint_resume_executors_internal,
    record_paused_checkpoint_resume_execution_plan_internal,
    record_paused_checkpoint_resume_executor_state_internal,
    record_paused_checkpoint_resume_work_item_internal,
};
use crate::repo::{
    cancel_one_task_for_user_chat, cancel_task_by_id, cancel_tasks_for_user_chat,
    get_task_admin_target,
};

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
            updated_at TEXT NOT NULL
        );",
    )
    .expect("create tasks table");
    drop(db);
    state
}

fn insert_task(
    state: &crate::AppState,
    task_id: &str,
    status: &str,
    result_json: Option<&serde_json::Value>,
    updated_at: i64,
) {
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, 42, 7, 'test-key', 'ui', 'ask', ?2, ?3, ?4, NULL, ?5, ?5)",
        rusqlite::params![
            task_id,
            json!({"text": "long task"}).to_string(),
            status,
            result_json.map(|value| value.to_string()),
            updated_at.to_string(),
        ],
    )
    .expect("insert task");
}

fn stored_result_json(state: &crate::AppState, task_id: &str) -> serde_json::Value {
    let db = state.core.db.get().expect("get db");
    let raw: String = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| row.get(0),
        )
        .expect("select result_json");
    serde_json::from_str(&raw).expect("parse result_json")
}

fn stored_task_status_and_error(
    state: &crate::AppState,
    task_id: &str,
) -> (String, Option<String>) {
    let db = state.core.db.get().expect("get db");
    db.query_row(
        "SELECT status, error_text FROM tasks WHERE task_id = ?1",
        rusqlite::params![task_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("select task status")
}

fn checkpoint_json(
    checkpoint_id: &str,
    completed_side_effect_refs: Vec<&str>,
) -> serde_json::Value {
    json!({
        "schema_version": 1,
        "checkpoint_id": checkpoint_id,
        "boundary_context": {"route_gate_kind": "execute"},
        "observations": [],
        "evidence_refs": [],
        "artifact_refs": [],
        "completed_side_effect_refs": completed_side_effect_refs,
        "budget": {
            "round": 1,
            "step": 1,
            "llm_calls": 1,
            "tool_calls": 0,
            "elapsed_ms": 10
        },
        "resume_entrypoint": "next_planner_round"
    })
}

#[test]
fn get_task_query_record_exposes_lifecycle_projection() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4();
    let progress = json!({
        "progress_messages": ["polling provider"],
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "resume_reason": "provider_gap_retry_window",
            "next_check_after": 1781800300,
            "checkpoint_id": "ckpt-query"
        }
    });
    insert_task(
        &state,
        &task_id.to_string(),
        "running",
        Some(&progress),
        1234,
    );

    let (response, _, _) = get_task_query_record(&state, task_id)
        .expect("query task")
        .expect("task exists");

    assert!(matches!(
        response.status,
        claw_core::types::TaskStatus::Running
    ));
    assert_eq!(
        response
            .result_json
            .as_ref()
            .and_then(|value| value.pointer("/task_lifecycle/state"))
            .and_then(serde_json::Value::as_str),
        Some("waiting")
    );
    let lifecycle = response.lifecycle.expect("lifecycle projection");
    assert_eq!(lifecycle["state"], "waiting");
    assert_eq!(lifecycle["db_status"], "running");
    assert_eq!(lifecycle["resume_reason"], "provider_gap_retry_window");
    assert_eq!(lifecycle["checkpoint_id"], "ckpt-query");
    assert_eq!(lifecycle["last_heartbeat_ts"], 1234);
}

#[test]
fn list_active_tasks_exposes_lifecycle_projection() {
    let state = state_with_tasks_table();
    let progress = json!({
        "task_lifecycle": {
            "state": "background",
            "resume_reason": "async_job_poll",
            "next_check_after": 1781800400,
            "checkpoint_id": "ckpt-active",
            "pending_job_ref": "job-17"
        }
    });
    insert_task(&state, "task-active-1", "running", Some(&progress), 2222);

    let tasks = list_active_tasks_internal(&state, 42, 7, None).expect("list active tasks");

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_id, "task-active-1");
    assert_eq!(tasks[0].status, "running");
    let lifecycle = tasks[0].lifecycle.as_ref().expect("lifecycle projection");
    assert_eq!(lifecycle["state"], "background");
    assert_eq!(lifecycle["resume_reason"], "async_job_poll");
    assert_eq!(lifecycle["pending_job_ref"], "job-17");
    assert_eq!(lifecycle["last_heartbeat_ts"], 2222);
}

#[test]
fn get_task_admin_target_and_cancel_task_by_id_use_machine_fields() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_task(&state, &task_id, "running", None, 1234);

    let target = get_task_admin_target(&state, &task_id)
        .expect("lookup target")
        .expect("target exists");
    assert_eq!(target.task_id, task_id);
    assert_eq!(target.user_id, 42);
    assert_eq!(target.chat_id, 7);
    assert_eq!(target.user_key.as_deref(), Some("test-key"));
    assert_eq!(target.channel, "ui");
    assert_eq!(target.status, "running");

    let canceled = cancel_task_by_id(&state, &target.task_id).expect("cancel task");
    assert_eq!(canceled, 1);
    let (status, error_text) = stored_task_status_and_error(&state, &task_id);
    assert_eq!(status, "canceled");
    assert_eq!(error_text.as_deref(), Some("user_cancelled"));
    let result = stored_result_json(&state, &task_id);
    assert_eq!(result["status_code"], "user_cancelled");
    assert_eq!(result["error_code"], "user_cancelled");
    assert_eq!(result["terminal_reason"], "user_cancelled");
    assert_eq!(result["message_key"], "clawd.task.cancelled");
    assert_eq!(result["task_lifecycle"]["state"], "cancelled");
    assert_eq!(
        result["task_lifecycle"]["terminal_reason"],
        "user_cancelled"
    );
    assert_eq!(
        result["task_lifecycle"]["message_key"],
        "clawd.task.cancelled"
    );
    assert_eq!(result["task_lifecycle"]["can_cancel"], false);
}

#[test]
fn cancel_task_by_id_does_not_touch_terminal_tasks() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_task(&state, &task_id, "succeeded", None, 1234);

    let canceled = cancel_task_by_id(&state, &task_id).expect("cancel task");
    assert_eq!(canceled, 0);
    let (status, error_text) = stored_task_status_and_error(&state, &task_id);
    assert_eq!(status, "succeeded");
    assert_eq!(error_text, None);
}

#[test]
fn cancel_one_task_for_user_chat_writes_machine_lifecycle() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_task(&state, &task_id, "queued", None, 1234);

    let canceled = cancel_one_task_for_user_chat(&state, 42, 7, &task_id).expect("cancel one task");

    assert_eq!(canceled, 1);
    let (status, error_text) = stored_task_status_and_error(&state, &task_id);
    assert_eq!(status, "canceled");
    assert_eq!(error_text.as_deref(), Some("user_cancelled"));
    let result = stored_result_json(&state, &task_id);
    assert_eq!(result["task_lifecycle"]["state"], "cancelled");
    assert_eq!(
        result["task_lifecycle"]["terminal_reason"],
        "user_cancelled"
    );
    assert_eq!(result["task_lifecycle"]["source"], "task_admin_cancel");
}

#[test]
fn cancel_tasks_for_user_chat_writes_machine_lifecycle_and_honors_exclude() {
    let state = state_with_tasks_table();
    let cancel_id = Uuid::new_v4().to_string();
    let excluded_id = Uuid::new_v4().to_string();
    insert_task(&state, &cancel_id, "running", None, 1234);
    insert_task(&state, &excluded_id, "running", None, 1234);

    let canceled =
        cancel_tasks_for_user_chat(&state, 42, 7, Some(&excluded_id)).expect("cancel tasks");

    assert_eq!(canceled, 1);
    let (status, error_text) = stored_task_status_and_error(&state, &cancel_id);
    assert_eq!(status, "canceled");
    assert_eq!(error_text.as_deref(), Some("user_cancelled"));
    let result = stored_result_json(&state, &cancel_id);
    assert_eq!(result["task_lifecycle"]["state"], "cancelled");
    assert_eq!(
        result["task_lifecycle"]["terminal_reason"],
        "user_cancelled"
    );
    let (excluded_status, excluded_error) = stored_task_status_and_error(&state, &excluded_id);
    assert_eq!(excluded_status, "running");
    assert_eq!(excluded_error, None);
}

#[test]
fn active_checkpoint_resume_context_uses_lifecycle_machine_fields() {
    let state = state_with_tasks_table();
    let now = 2_000;
    let checkpoint = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "needs_user",
            "resume_reason": "await_user_input",
            "checkpoint_id": "ckpt-active",
            "resume_executor": {
                "executor_state": "awaiting_user",
                "checkpoint_id": "ckpt-active",
                "resume_directive": "await_user_input"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-active", vec![])
    });
    insert_task(&state, "active-resume", "running", Some(&checkpoint), now);

    let candidate = crate::repo::find_active_checkpoint_resume_context(&state, 42, 7)
        .expect("active checkpoint context");

    assert_eq!(candidate.resume_context["updated_ts"], now);
    assert_eq!(
        candidate.resume_context["source"],
        "active_checkpoint_resume"
    );
    assert_eq!(candidate.resume_context["task_id"], "active-resume");
    assert_eq!(
        candidate.resume_context["task_lifecycle"]["state"],
        "needs_user"
    );
    assert_eq!(
        candidate.resume_context["task_lifecycle"]["resume_executor"]["executor_state"],
        "awaiting_user"
    );
    assert_eq!(
        candidate.resume_context["task_checkpoint"]["checkpoint_id"],
        "ckpt-active"
    );
    assert!(candidate.resume_context.get("text").is_none());
    assert!(candidate.resume_context.get("error_text").is_none());
}

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

    insert_task(
        &state,
        "due-journal",
        "running",
        Some(&due_from_journal),
        10,
    );
    insert_task(&state, "future-wait", "running", Some(&future_wait), 20);
    insert_task(&state, "invalid", "running", Some(&invalid_checkpoint), 30);
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
    assert_eq!(lifecycle["resume_claim"]["owner"], "worker_recovery");
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
}

#[test]
fn list_ready_paused_checkpoint_resume_executors_filters_machine_states() {
    let state = state_with_tasks_table();
    let now = 3_000;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "next_check_after": now,
            "checkpoint_id": "ckpt-ready",
            "resume_work_item": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-ready",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            },
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-ready",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-ready", vec!["write_file:tmp/report.txt"])
    });
    let due_poll = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "async_job_poll",
            "next_check_after": now - 1,
            "checkpoint_id": "ckpt-poll",
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-poll",
                "executor_state": "poll_scheduled",
                "resume_trigger": "worker_recovery",
                "resume_directive": "poll_async_job",
                "job_id": "job-1"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-poll", vec![])
    });
    let future = json!({
        "task_lifecycle": {
            "state": "background",
            "next_check_after": now + 60,
            "checkpoint_id": "ckpt-future",
            "resume_executor": {
                "checkpoint_id": "ckpt-future",
                "executor_state": "ready_to_finalize",
                "resume_trigger": "worker_recovery",
                "resume_directive": "verify_and_finalize"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-future", vec![])
    });
    let awaiting_user = json!({
        "task_lifecycle": {
            "state": "needs_user",
            "checkpoint_id": "ckpt-user",
            "resume_executor": {
                "checkpoint_id": "ckpt-user",
                "executor_state": "awaiting_user",
                "resume_trigger": "worker_recovery",
                "resume_directive": "await_user_input"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-user", vec![])
    });
    let mismatched_executor = json!({
        "task_lifecycle": {
            "state": "background",
            "next_check_after": now,
            "checkpoint_id": "ckpt-mismatch",
            "resume_executor": {
                "checkpoint_id": "ckpt-other",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-mismatch", vec![])
    });
    let missing_directive = json!({
        "task_lifecycle": {
            "state": "background",
            "next_check_after": now,
            "checkpoint_id": "ckpt-missing-directive",
            "resume_executor": {
                "checkpoint_id": "ckpt-missing-directive",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-missing-directive", vec![])
    });
    let checkpoint_mismatch = json!({
        "task_lifecycle": {
            "state": "background",
            "next_check_after": now,
            "checkpoint_id": "ckpt-lifecycle",
            "resume_executor": {
                "checkpoint_id": "ckpt-lifecycle",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-task", vec![])
    });

    insert_task(&state, "ready-planner", "running", Some(&ready_planner), 10);
    insert_task(&state, "due-poll", "running", Some(&due_poll), 20);
    insert_task(&state, "future", "running", Some(&future), 30);
    insert_task(&state, "awaiting-user", "running", Some(&awaiting_user), 40);
    insert_task(
        &state,
        "mismatched-executor",
        "running",
        Some(&mismatched_executor),
        50,
    );
    insert_task(
        &state,
        "missing-directive",
        "running",
        Some(&missing_directive),
        60,
    );
    insert_task(
        &state,
        "checkpoint-mismatch",
        "running",
        Some(&checkpoint_mismatch),
        70,
    );

    let first =
        list_ready_paused_checkpoint_resume_executors_internal(&state, now, 1).expect("list first");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].task_id, "ready-planner");
    assert_eq!(first[0].checkpoint_id, "ckpt-ready");
    assert_eq!(first[0].executor_state, "ready_for_planner_resume");
    assert_eq!(first[0].resume_trigger, "worker_recovery");
    assert_eq!(first[0].resume_directive, "run_next_planner_round");
    assert_eq!(first[0].next_check_after, Some(now));
    assert_eq!(first[0].task_checkpoint.completed_side_effect_refs.len(), 1);
    assert_eq!(
        first[0]
            .resume_work_item
            .as_ref()
            .and_then(|value| value.get("resume_trigger"))
            .and_then(serde_json::Value::as_str),
        Some("worker_recovery")
    );

    let all = list_ready_paused_checkpoint_resume_executors_internal(&state, now, 10)
        .expect("list ready executors");
    let task_ids: Vec<_> = all.iter().map(|task| task.task_id.as_str()).collect();
    assert_eq!(task_ids, vec!["ready-planner", "due-poll"]);
    assert_eq!(all[1].executor_state, "poll_scheduled");
    assert_eq!(all[1].resume_directive, "poll_async_job");
    assert_eq!(all[1].resume_executor["job_id"], "job-1");
}

#[test]
fn claim_ready_paused_checkpoint_resume_executor_sets_machine_lease() {
    let state = state_with_tasks_table();
    let now = 4_000;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "resume_due": true,
            "resume_wait_seconds": 0,
            "next_check_after": now,
            "checkpoint_id": "ckpt-ready",
            "resume_claim": {
                "schema_version": 1,
                "owner": "worker_recovery",
                "checkpoint_id": "ckpt-ready",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_work_item": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-ready",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-ready",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-ready", vec!["write_file:tmp/report.txt"])
    });
    insert_task(&state, "ready-planner", "running", Some(&ready_planner), 10);

    assert!(
        claim_ready_paused_checkpoint_resume_executor_internal(
            &state,
            "ready-planner",
            "ckpt-other",
            "ready_for_planner_resume",
            now + 1,
            30,
        )
        .expect("claim wrong checkpoint")
        .is_none(),
        "checkpoint mismatch must not claim"
    );
    assert!(
        claim_ready_paused_checkpoint_resume_executor_internal(
            &state,
            "ready-planner",
            "ckpt-ready",
            "ready_to_finalize",
            now + 1,
            30,
        )
        .expect("claim wrong state")
        .is_none(),
        "executor-state mismatch must not claim"
    );

    let claimed = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        "ready-planner",
        "ckpt-ready",
        "ready_for_planner_resume",
        now + 1,
        30,
    )
    .expect("claim ready executor")
    .expect("executor claimed");
    assert_eq!(claimed.task_id, "ready-planner");
    assert_eq!(claimed.task.task_id, "ready-planner");
    assert_eq!(claimed.task.user_id, 42);
    assert_eq!(claimed.task.chat_id, 7);
    assert_eq!(claimed.task.channel, "ui");
    assert_eq!(claimed.task.kind, "ask");
    assert!(
        claimed.task.payload_json.contains("long task"),
        "claimed executor must carry original task payload for seeded replay"
    );
    assert_eq!(claimed.checkpoint_id, "ckpt-ready");
    assert_eq!(claimed.previous_executor_state, "ready_for_planner_resume");
    assert_eq!(claimed.executor_state, "executing_planner_resume");
    assert_eq!(claimed.resume_trigger, "worker_recovery");
    assert_eq!(claimed.resume_directive, "run_next_planner_round");
    assert_eq!(claimed.lease_expires_at, now + 31);
    assert_eq!(
        claimed.resume_executor["executor_state"],
        "executing_planner_resume"
    );
    assert_eq!(
        claimed
            .resume_work_item
            .as_ref()
            .and_then(|value| value.get("executor_state"))
            .and_then(serde_json::Value::as_str),
        Some("executing_planner_resume")
    );
    assert_eq!(claimed.task_checkpoint.completed_side_effect_refs.len(), 1);

    let stored = stored_result_json(&state, "ready-planner");
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&stored), None);
    assert_eq!(lifecycle["state"], "running");
    assert_eq!(lifecycle["resume_due"], false);
    assert_eq!(lifecycle["resume_wait_seconds"], 0);
    assert_eq!(
        lifecycle["resume_executor"]["previous_executor_state"],
        "ready_for_planner_resume"
    );
    assert_eq!(
        lifecycle["resume_executor"]["executor_state"],
        "executing_planner_resume"
    );
    assert_eq!(lifecycle["resume_executor"]["executor_state_at"], now + 1);
    assert_eq!(
        lifecycle["resume_executor"]["executor_claim_expires_at"],
        now + 31
    );
    assert_eq!(
        lifecycle["resume_executor_claim"]["owner"],
        "worker_recovery_executor"
    );
    assert_eq!(
        lifecycle["resume_executor_claim"]["previous_executor_state"],
        "ready_for_planner_resume"
    );
    assert_eq!(
        lifecycle["resume_executor_claim"]["executor_state"],
        "executing_planner_resume"
    );
    assert_eq!(lifecycle["resume_executor_claim"]["expires_at"], now + 31);
    assert_eq!(
        lifecycle["resume_claim"]["executor_state"],
        "executing_planner_resume"
    );
    assert_eq!(
        lifecycle["resume_work_item"]["executor_state"],
        "executing_planner_resume"
    );

    let during_lease = list_ready_paused_checkpoint_resume_executors_internal(&state, now + 10, 10)
        .expect("list during executor lease");
    assert!(
        during_lease.is_empty(),
        "active executor lease must suppress duplicate ready claims"
    );

    let after_expiry = list_ready_paused_checkpoint_resume_executors_internal(&state, now + 31, 10)
        .expect("list after executor lease expiry");
    assert_eq!(after_expiry.len(), 1);
    assert_eq!(after_expiry[0].task_id, "ready-planner");
    assert_eq!(after_expiry[0].executor_state, "executing_planner_resume");

    let reclaimed = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        "ready-planner",
        "ckpt-ready",
        "executing_planner_resume",
        now + 32,
        20,
    )
    .expect("reclaim expired executor")
    .expect("expired executor should be reclaimable");
    assert_eq!(
        reclaimed.previous_executor_state,
        "executing_planner_resume"
    );
    assert_eq!(reclaimed.executor_state, "executing_planner_resume");
    assert_eq!(reclaimed.lease_expires_at, now + 52);
}

#[test]
fn record_paused_checkpoint_resume_execution_plan_requires_active_executor_claim() {
    let state = state_with_tasks_table();
    let now = 5_000;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "resume_due": true,
            "resume_wait_seconds": 0,
            "next_check_after": now,
            "checkpoint_id": "ckpt-plan",
            "resume_claim": {
                "schema_version": 1,
                "owner": "worker_recovery",
                "checkpoint_id": "ckpt-plan",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_work_item": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-plan",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round",
                "executor_state": "ready_for_planner_resume"
            },
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-plan",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-plan", vec!["write_file:tmp/report.txt"])
    });
    insert_task(&state, "ready-plan", "running", Some(&ready_planner), 10);

    let claimed = claim_ready_paused_checkpoint_resume_executor_internal(
        &state,
        "ready-plan",
        "ckpt-plan",
        "ready_for_planner_resume",
        now + 1,
        30,
    )
    .expect("claim ready executor")
    .expect("executor claimed");
    let plan_payload = json!({
        "schema_version": 1,
        "task_id": claimed.task_id,
        "checkpoint_id": claimed.checkpoint_id,
        "executor_action": "run_seeded_agent_loop",
        "executor_state": claimed.executor_state,
        "resume_directive": claimed.resume_directive,
        "resume_trigger": claimed.resume_trigger,
        "completed_side_effect_count": 1,
        "requires_idempotency_guard": true
    });

    assert!(
        !record_paused_checkpoint_resume_execution_plan_internal(
            &state,
            "ready-plan",
            "ckpt-other",
            "executing_planner_resume",
            &plan_payload,
            now + 2,
        )
        .expect("record wrong checkpoint"),
        "checkpoint mismatch must not persist execution plan"
    );
    assert!(
        record_paused_checkpoint_resume_execution_plan_internal(
            &state,
            "ready-plan",
            "ckpt-plan",
            "executing_planner_resume",
            &plan_payload,
            now + 2,
        )
        .expect("record execution plan"),
        "active executor claim should accept execution plan"
    );

    let stored = stored_result_json(&state, "ready-plan");
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection("running", Some(&stored), None);
    assert_eq!(
        lifecycle["resume_execution_plan"]["executor_action"],
        "run_seeded_agent_loop"
    );
    assert_eq!(lifecycle["resume_execution_plan"]["planned_at"], now + 2);
    assert_eq!(
        lifecycle["resume_executor"]["execution_plan_action"],
        "run_seeded_agent_loop"
    );
    assert_eq!(
        lifecycle["resume_executor_claim"]["execution_plan_action"],
        "run_seeded_agent_loop"
    );
    assert!(lifecycle["resume_execution_plan"].get("text").is_none());
    assert!(lifecycle["resume_execution_plan"]
        .get("error_text")
        .is_none());
}

#[path = "task_resume_execution_tests.rs"]
mod task_resume_execution_tests;

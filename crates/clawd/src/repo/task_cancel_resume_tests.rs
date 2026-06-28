use serde_json::json;

use super::{
    claim_ready_paused_checkpoint_resume_executor_internal,
    list_ready_paused_checkpoint_resume_executors_internal,
};
use crate::repo::cancel_task_by_id;

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
            "tool_calls": 1,
            "elapsed_ms": 120
        },
        "resume_entrypoint": "next_planner_round"
    })
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
            json!({"request_kind": "resume_boundary_test"}).to_string(),
            status,
            result_json.map(|value| value.to_string()),
            updated_at.to_string(),
        ],
    )
    .expect("insert task");
}

fn stored_status(state: &crate::AppState, task_id: &str) -> String {
    let db = state.core.db.get().expect("get db");
    db.query_row(
        "SELECT status FROM tasks WHERE task_id = ?1",
        rusqlite::params![task_id],
        |row| row.get(0),
    )
    .expect("select task status")
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

#[test]
fn canceled_ready_paused_checkpoint_is_not_resumed_or_claimed() {
    let state = state_with_tasks_table();
    let now = 3_500;
    let ready_planner = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "background",
            "resume_reason": "agent_loop_soft_budget",
            "next_check_after": now,
            "checkpoint_id": "ckpt-cancel-ready",
            "resume_work_item": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-cancel-ready",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            },
            "resume_executor": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-cancel-ready",
                "executor_state": "ready_for_planner_resume",
                "resume_trigger": "worker_recovery",
                "resume_directive": "run_next_planner_round"
            }
        },
        "task_checkpoint": checkpoint_json("ckpt-cancel-ready", vec!["write_file:tmp/report.txt"])
    });
    insert_task(&state, "cancel-ready", "running", Some(&ready_planner), 80);

    let ready_before = list_ready_paused_checkpoint_resume_executors_internal(&state, now, 10)
        .expect("list ready before cancel");
    assert_eq!(ready_before.len(), 1);
    assert_eq!(ready_before[0].task_id, "cancel-ready");

    let canceled = cancel_task_by_id(&state, "cancel-ready").expect("cancel ready task");
    assert_eq!(canceled, 1);

    assert!(
        list_ready_paused_checkpoint_resume_executors_internal(&state, now, 10)
            .expect("list ready after cancel")
            .is_empty(),
        "terminal cancellation must suppress paused checkpoint resume dispatch"
    );
    assert!(
        claim_ready_paused_checkpoint_resume_executor_internal(
            &state,
            "cancel-ready",
            "ckpt-cancel-ready",
            "ready_for_planner_resume",
            now + 1,
            30,
        )
        .expect("claim after cancel")
        .is_none(),
        "terminal cancellation must suppress direct resume claims"
    );
    assert_eq!(stored_status(&state, "cancel-ready"), "canceled");
    let result = stored_result_json(&state, "cancel-ready");
    assert_eq!(result["status_code"], "user_cancelled");
    assert_eq!(result["task_lifecycle"]["state"], "cancelled");
    assert_eq!(
        result["task_lifecycle"]["terminal_reason"],
        "user_cancelled"
    );
    assert_eq!(
        result["task_lifecycle"]["message_key"],
        "clawd.task.cancelled"
    );
}

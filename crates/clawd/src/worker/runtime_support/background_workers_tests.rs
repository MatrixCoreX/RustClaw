use serde_json::{json, Value};
use uuid::Uuid;

use super::*;

fn test_state() -> crate::AppState {
    crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema()
}

fn checkpoint_result(checkpoint_id: &str, next_check_after: i64) -> Value {
    json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "checkpoint_id": checkpoint_id,
            "next_check_after": next_check_after,
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "boundary_context": {"route_gate_kind": "execute"},
            "observations": [],
            "evidence_refs": [],
            "artifact_refs": [],
            "completed_side_effect_refs": [],
            "budget": {
                "round": 1,
                "step": 1,
                "llm_calls": 1,
                "tool_calls": 0,
                "elapsed_ms": 100,
            },
            "resume_entrypoint": "next_planner_round",
        }
    })
}

fn insert_task(state: &AppState, task_id: &str, status: &str, result: Option<&Value>) {
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at,
            lease_owner, lease_expires_at, claim_attempt, claimed_at
        ) VALUES (
            ?1, 42, 7, 'test-key', 'ui', 'ask', ?2,
            ?3, ?4, NULL, '1', '1', NULL, 0, 0, 0
        )",
        rusqlite::params![
            task_id,
            json!({"text": "scheduled fixture"}).to_string(),
            status,
            result.map(Value::to_string),
        ],
    )
    .expect("insert task");
}

fn insert_due_job(state: &AppState, job_id: &str, task_id: &str, next_run_at: i64) {
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO scheduled_jobs (
            job_id, user_id, chat_id, user_key, channel, schedule_type,
            every_minutes, timezone, task_kind, task_payload_json, enabled,
            notify_on_success, notify_on_failure, next_run_at, isolation_profile,
            permission_policy_json, thread_resume_enabled, last_thread_task_id,
            created_at, updated_at
        ) VALUES (
            ?1, 42, 7, 'test-key', 'ui', 'interval',
            5, 'UTC', 'ask', ?2, 1,
            1, 1, ?3, 'local_current_workspace',
            '{}', 1, ?4, '1', '1'
        )",
        rusqlite::params![
            job_id,
            json!({"text": "scheduled fixture"}).to_string(),
            next_run_at,
            task_id,
        ],
    )
    .expect("insert scheduled job");
}

fn task_result(state: &AppState, task_id: &str) -> Value {
    let db = state.core.db.get().expect("get db");
    let raw: String = db
        .query_row(
            "SELECT result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| row.get(0),
        )
        .expect("read task result");
    serde_json::from_str(&raw).expect("parse task result")
}

#[test]
fn cleanup_removes_cost_ledger_rows_after_task_retention_removes_owner() {
    let state = test_state();
    let task_id = format!("task-cost-cleanup-{}", Uuid::new_v4().simple());
    insert_task(&state, &task_id, "succeeded", Some(&json!({})));
    {
        let db = state.core.db.get().expect("get db");
        db.execute(
            "INSERT INTO llm_cost_ledger (
                task_id, user_id, provider, model, logical_call_index, prompt_label,
                provider_status, cost_status, estimated_cost_usd_nanos, record_json,
                created_at_ts
             ) VALUES (?1, 42, 'vendor-primary', 'model-a', 1, 'plan',
                       'ok', 'known', 1000, '{}', 1)",
            rusqlite::params![task_id],
        )
        .expect("insert cost ledger row");
    }

    cleanup_once(&state).expect("run cleanup");

    let db = state.core.db.get().expect("get db");
    let task_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE task_id=?1",
            rusqlite::params![task_id],
            |row| row.get(0),
        )
        .expect("count retained task");
    let cost_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM llm_cost_ledger WHERE task_id=?1",
            rusqlite::params![task_id],
            |row| row.get(0),
        )
        .expect("count retained cost rows");
    assert_eq!(task_count, 0);
    assert_eq!(cost_count, 0);
}

#[test]
fn scheduled_wakeup_resumes_waiting_thread_without_enqueuing_duplicate_task() {
    let state = test_state();
    let now = crate::now_ts_u64() as i64;
    let task_id = Uuid::new_v4().to_string();
    let job_id = format!("job_{}", Uuid::new_v4().simple());
    insert_task(
        &state,
        &task_id,
        "running",
        Some(&checkpoint_result("ckpt-scheduled-wakeup", now + 3_600)),
    );
    insert_due_job(&state, &job_id, &task_id, now - 1);

    schedule_once(&state).expect("run schedule worker");

    let db = state.core.db.get().expect("get db");
    let task_count: i64 = db
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .expect("count tasks");
    let (next_run_at, last_thread_task_id): (i64, String) = db
        .query_row(
            "SELECT next_run_at, last_thread_task_id
             FROM scheduled_jobs WHERE job_id = ?1",
            rusqlite::params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read scheduled job");
    drop(db);

    assert_eq!(task_count, 1);
    assert!(next_run_at > now);
    assert_eq!(last_thread_task_id, task_id);
    let result = task_result(&state, &task_id);
    assert_eq!(
        result["task_lifecycle"]["resume_input"]["resume_trigger"],
        "scheduled_wakeup"
    );
    assert_eq!(
        result["task_lifecycle"]["control_request"]["trigger"],
        "scheduled_wakeup"
    );
    let due = repo::list_due_paused_checkpoint_tasks_internal(&state, now + 1, 10)
        .expect("list due checkpoints");
    assert_eq!(due.len(), 1);
    assert_eq!(
        due[0].resume_trigger,
        crate::task_lifecycle::ResumeTrigger::ScheduledWakeup
    );
}

#[test]
fn scheduled_wakeup_enqueues_new_thread_after_previous_task_is_terminal() {
    let state = test_state();
    let now = crate::now_ts_u64() as i64;
    let previous_task_id = Uuid::new_v4().to_string();
    let job_id = format!("job_{}", Uuid::new_v4().simple());
    insert_task(&state, &previous_task_id, "succeeded", Some(&json!({})));
    insert_due_job(&state, &job_id, &previous_task_id, now - 1);

    schedule_once(&state).expect("run schedule worker");

    let db = state.core.db.get().expect("get db");
    let task_count: i64 = db
        .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
        .expect("count tasks");
    let next_task_id: String = db
        .query_row(
            "SELECT last_thread_task_id FROM scheduled_jobs WHERE job_id = ?1",
            rusqlite::params![job_id],
            |row| row.get(0),
        )
        .expect("read next thread task");
    let next_payload_raw: String = db
        .query_row(
            "SELECT payload_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![next_task_id],
            |row| row.get(0),
        )
        .expect("read next task payload");
    let run_count: i64 = db
        .query_row("SELECT COUNT(*) FROM scheduled_job_runs", [], |row| {
            row.get(0)
        })
        .expect("count scheduled runs");
    let next_payload: Value =
        serde_json::from_str(&next_payload_raw).expect("parse next task payload");

    assert_eq!(task_count, 2);
    assert_ne!(next_task_id, previous_task_id);
    assert_eq!(
        next_payload["thread_resume_task_id"],
        previous_task_id.as_str()
    );
    assert_eq!(next_payload["resume_trigger"], "scheduled_wakeup");
    assert_eq!(run_count, 1);
}

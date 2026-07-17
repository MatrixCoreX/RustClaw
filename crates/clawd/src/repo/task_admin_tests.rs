use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::Duration;
use uuid::Uuid;

use super::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw-task-admin-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("create temp directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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

fn file_backed_state_with_schema(db_path: &Path) -> crate::AppState {
    let manager = r2d2_sqlite::SqliteConnectionManager::file(db_path).with_init(
        |conn: &mut rusqlite::Connection| {
            conn.busy_timeout(Duration::from_secs(5))?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        },
    );
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.db = r2d2::Pool::builder()
        .max_size(8)
        .build(manager)
        .expect("build file-backed test pool");
    state.worker.database_sqlite_path = db_path.to_path_buf();
    state.with_seeded_db_schema()
}

fn insert_running_task(state: &crate::AppState, task_id: &str, result_json: &Value) {
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, 42, 7, 'test-key', 'ui', 'ask', ?2, 'running', ?3, NULL, '1', '1')",
        rusqlite::params![
            task_id,
            json!({"text": "visible request"}).to_string(),
            result_json.to_string(),
        ],
    )
    .expect("insert task");
}

fn stored_result_json(state: &crate::AppState, task_id: &str) -> Value {
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

fn paused_checkpoint_result(checkpoint_id: &str, next_check_after: i64) -> Value {
    json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "resume_reason": "agent_loop_max_rounds",
            "next_check_after": next_check_after,
            "checkpoint_id": checkpoint_id
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
                "elapsed_ms": 100
            },
            "resume_entrypoint": "next_planner_round"
        }
    })
}

#[test]
fn cancel_task_by_id_records_provider_cancel_contract_without_text_fields() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_running_task(
        &state,
        &task_id,
        &json!({
            "task_checkpoint": {
                "pending_async_job": {
                    "job_id": "provider:video_generate:minimax:task-1",
                    "status": "running",
                    "poll_after_seconds": 30,
                    "expires_at": 9_999,
                    "cancel_ref": "provider:video_generate:minimax:task-1",
                    "message_key": "clawd.task.async_job_pending",
                    "poll_adapter": {
                        "adapter_kind": "media_job_poll",
                        "skill_name": "video_generate"
                    }
                }
            }
        }),
    );

    let canceled = cancel_task_by_id(&state, &task_id).expect("cancel task");

    assert_eq!(canceled, 1);
    let result = stored_result_json(&state, &task_id);
    assert_eq!(
        result["cancel_adapter_result"]["adapter_kind"],
        "media_job_poll"
    );
    assert_eq!(
        result["cancel_adapter_result"]["status"],
        "requires_provider_adapter"
    );
    assert_eq!(
        result["cancel_adapter_result"]["error_code"],
        "provider_cancel_adapter_missing"
    );
    assert_eq!(
        result["cancel_adapter_result"]["provider_cancel_contract"]["provider"],
        "minimax"
    );
    assert_eq!(
        result["cancel_adapter_result"]["provider_cancel_contract"]["job_id"],
        "task-1"
    );
    assert_eq!(
        result["task_lifecycle"]["cancel_adapter_kind"],
        "media_job_poll"
    );
    assert!(result["cancel_adapter_result"].get("text").is_none());
    assert!(result["cancel_adapter_result"].get("error_text").is_none());
}

#[test]
fn cancel_task_by_id_accepts_provider_cancel_token_alias() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_running_task(
        &state,
        &task_id,
        &json!({
            "task_checkpoint": {
                "pending_async_job": {
                    "job_id": "provider:video_generate:minimax:task-1",
                    "status": "running",
                    "poll_after_ms": 30_000,
                    "expires_at": 9_999,
                    "cancel_token": "provider:video_generate:minimax:task-1",
                    "message_key": "clawd.task.async_job_pending",
                    "poll_adapter": {
                        "adapter_kind": "media_job_poll",
                        "skill_name": "video_generate"
                    }
                }
            }
        }),
    );

    let canceled = cancel_task_by_id(&state, &task_id).expect("cancel task");

    assert_eq!(canceled, 1);
    let result = stored_result_json(&state, &task_id);
    assert_eq!(
        result["cancel_adapter_result"]["provider_cancel_contract"]["provider"],
        "minimax"
    );
    assert_eq!(
        result["cancel_adapter_result"]["provider_cancel_contract"]["job_id"],
        "task-1"
    );
    assert_eq!(
        result["cancel_adapter_result"]["adapter_kind"],
        "media_job_poll"
    );
    assert!(result["cancel_adapter_result"].get("text").is_none());
    assert!(result["cancel_adapter_result"].get("error_text").is_none());
}

#[test]
fn cancel_task_by_id_signals_the_matching_active_runtime_only() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    let other_task_id = Uuid::new_v4().to_string();
    insert_running_task(&state, &task_id, &json!({}));
    insert_running_task(&state, &other_task_id, &json!({}));
    let token = state.worker.register_active_task(&task_id);
    let other_token = state.worker.register_active_task(&other_task_id);

    assert_eq!(cancel_task_by_id(&state, &task_id).expect("cancel task"), 1);
    assert!(token.is_cancelled());
    assert!(!other_token.is_cancelled());

    state.worker.unregister_active_task(&task_id);
    state.worker.unregister_active_task(&other_task_id);
}

#[test]
fn resume_task_with_input_records_structured_resume_metadata() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_running_task(
        &state,
        &task_id,
        &paused_checkpoint_result("ckpt-resume", 1),
    );

    let update = resume_task_with_input(
        &state,
        TaskResumeControlInput {
            task_id: task_id.clone(),
            checkpoint_id: Some("ckpt-resume".to_string()),
            resume_reason: Some("manual_resume".to_string()),
            user_message: Some("continue with tighter budget".to_string()),
            new_constraints: Some(json!({"budget_profile": "short"})),
        },
    )
    .expect("resume task")
    .expect("resume update");

    assert_eq!(update.checkpoint_id, "ckpt-resume");
    let result = stored_result_json(&state, &task_id);
    let lifecycle = &result["task_lifecycle"];
    assert_eq!(lifecycle["source"], "task_admin_control");
    assert_eq!(lifecycle["message_key"], "clawd.task.resume_requested");
    assert_eq!(lifecycle["manual_control_kind"], "resume");
    assert_eq!(lifecycle["manual_control_status"], "pending");
    assert_eq!(lifecycle["resume_due"], true);
    assert_eq!(lifecycle["resume_input"]["task_id"], task_id);
    assert_eq!(lifecycle["resume_input"]["checkpoint_id"], "ckpt-resume");
    assert_eq!(lifecycle["resume_input"]["resume_reason"], "manual_resume");
    assert_eq!(lifecycle["resume_input"]["user_message_present"], true);
    assert_eq!(lifecycle["resume_input"]["user_message_char_count"], 28);
    assert_eq!(
        lifecycle["resume_input"]["new_constraints"]["budget_profile"],
        "short"
    );
    assert!(lifecycle["resume_input"].get("text").is_none());
    assert!(lifecycle["resume_input"].get("error_text").is_none());
}

#[test]
fn resume_task_with_input_rejects_checkpoint_mismatch() {
    let state = state_with_tasks_table();
    let task_id = Uuid::new_v4().to_string();
    insert_running_task(&state, &task_id, &paused_checkpoint_result("ckpt-real", 1));

    let update = resume_task_with_input(
        &state,
        TaskResumeControlInput {
            task_id,
            checkpoint_id: Some("ckpt-other".to_string()),
            resume_reason: Some("manual_resume".to_string()),
            user_message: None,
            new_constraints: None,
        },
    )
    .expect("resume task");

    assert!(update.is_none());
}

#[test]
fn concurrent_duplicate_resume_requests_have_one_accepted_owner() {
    let temp = TempDir::new();
    let state = file_backed_state_with_schema(&temp.0.join("tasks.sqlite"));
    let task_id = Uuid::new_v4().to_string();
    insert_running_task(
        &state,
        &task_id,
        &paused_checkpoint_result("ckpt-concurrent-resume", 1),
    );
    let barrier = Arc::new(Barrier::new(8));
    let mut threads = Vec::new();

    for _ in 0..8 {
        let state = state.clone();
        let task_id = task_id.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            resume_task_with_input(
                &state,
                TaskResumeControlInput {
                    task_id,
                    checkpoint_id: Some("ckpt-concurrent-resume".to_string()),
                    resume_reason: Some("manual_resume".to_string()),
                    user_message: None,
                    new_constraints: None,
                },
            )
            .expect("concurrent resume request")
        }));
    }

    let outcomes = threads
        .into_iter()
        .map(|thread| thread.join().expect("join resume request"))
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_some()).count(),
        1
    );
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_none()).count(),
        7
    );
}

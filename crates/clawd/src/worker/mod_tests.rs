use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::json;

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_{prefix}_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn memory_tasks_db() -> rusqlite::Connection {
    let db = rusqlite::Connection::open_in_memory().expect("open memory db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
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
    db
}

fn state_with_runtime_tasks_table() -> crate::AppState {
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
    .expect("create runtime tasks table");
    drop(db);
    state
}

fn file_db_pool(path: &Path) -> crate::db_init::DbPool {
    let manager = r2d2_sqlite::SqliteConnectionManager::file(path).with_init(
        |conn: &mut rusqlite::Connection| {
            conn.busy_timeout(Duration::from_millis(5_000))?;
            conn.pragma_update(None, "journal_mode", "WAL")?;
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        },
    );
    r2d2::Pool::builder()
        .max_size(2)
        .build(manager)
        .expect("build file-backed test db pool")
}

fn file_backed_state_with_schema(db_path: &Path) -> crate::AppState {
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.db = file_db_pool(db_path);
    state.worker.database_sqlite_path = db_path.to_path_buf();
    state.with_seeded_db_schema()
}

fn valid_checkpoint_json(checkpoint_id: &str, resume_entrypoint: &str) -> serde_json::Value {
    json!({
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
            "elapsed_ms": 10
        },
        "resume_entrypoint": resume_entrypoint
    })
}

#[test]
fn wechat_payload_shape_keeps_context_token_available() {
    let payload = json!({
        "channel": "wechat",
        "external_chat_id": "wx-user-1",
        "context_token": "ctx-123"
    });
    assert_eq!(
        payload.get("channel").and_then(|v| v.as_str()),
        Some("wechat")
    );
    assert_eq!(
        payload.get("context_token").and_then(|v| v.as_str()),
        Some("ctx-123")
    );
}

#[test]
fn startup_recovery_times_out_only_stale_running_tasks() {
    let db = memory_tasks_db();
    let recent_ts = crate::now_ts_u64() as i64;
    let old_ts = 1_i64;
    for (task_id, status, updated_at) in [
        ("running-old", "running", old_ts),
        ("running-recent", "running", recent_ts),
        ("queued-old", "queued", old_ts),
        ("succeeded-old", "succeeded", old_ts),
    ] {
        db.execute(
            "INSERT INTO tasks (task_id, status, error_text, created_at, updated_at)
             VALUES (?1, ?2, NULL, ?3, ?3)",
            rusqlite::params![task_id, status, updated_at.to_string()],
        )
        .expect("insert task");
    }

    let recovered =
        super::recover_stale_running_tasks_on_startup(&db, 60).expect("recover stale running");

    assert_eq!(recovered, vec!["running-old".to_string()]);
    let rows = [
        "running-old",
        "running-recent",
        "queued-old",
        "succeeded-old",
    ]
    .into_iter()
    .map(|task_id| {
        db.query_row(
            "SELECT status, error_text FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| {
                Ok((
                    task_id,
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .expect("query task")
    })
    .collect::<Vec<_>>();

    assert_eq!(rows[0].1, "timeout");
    assert_eq!(rows[0].2.as_deref(), Some("worker_heartbeat_stale"));
    assert_eq!(rows[1].1, "running");
    assert_eq!(rows[2].1, "queued");
    assert_eq!(rows[3].1, "succeeded");
}

#[test]
fn startup_recovery_times_out_expired_worker_lease() {
    let db = memory_tasks_db();
    let now = crate::now_ts_u64() as i64;
    db.execute(
        "INSERT INTO tasks (
            task_id, status, error_text, created_at, updated_at,
            lease_owner, lease_expires_at, claim_attempt, claimed_at
        )
        VALUES (?1, 'running', NULL, ?2, ?2, 'worker:test-old', ?3, 1, ?4)",
        rusqlite::params![
            "lease-expired",
            now.to_string(),
            now.saturating_sub(1),
            now.saturating_sub(120),
        ],
    )
    .expect("insert expired lease task");

    let recovered =
        super::recover_stale_running_tasks_on_startup(&db, 60).expect("recover stale running");

    assert_eq!(recovered, vec!["lease-expired".to_string()]);
    let (status, error_text): (String, Option<String>) = db
        .query_row(
            "SELECT status, error_text FROM tasks WHERE task_id = ?1",
            rusqlite::params!["lease-expired"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query task");
    assert_eq!(status, "timeout");
    assert_eq!(error_text.as_deref(), Some("worker_lease_expired"));
}

#[test]
fn startup_recovery_preserves_terminal_projection_pending_state() {
    let db = memory_tasks_db();
    let old_ts = 1_i64;
    let pending_projection = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "running",
            "checkpoint_id": "ckpt-terminal-pending",
            "resume_executor_dispatch_result": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-terminal-pending",
                "executor_state": "executing_finalize",
                "executor_action": "verify_and_finalize",
                "executor_status": "finalize_ready",
                "dispatch_state": "ready_to_verify_and_finalize",
                "executor_result_status": "finalize_completed",
                "result_projection_state": "project_finalize_completed",
                "projection_pending_reason": "terminal_projection_pending",
                "recorded_at": old_ts
            }
        },
        "task_checkpoint": valid_checkpoint_json("ckpt-terminal-pending", "verify_and_finalize")
    });
    db.execute(
        "INSERT INTO tasks (task_id, status, result_json, error_text, created_at, updated_at)
         VALUES ('terminal-projection-pending', 'running', ?1, NULL, ?2, ?2)",
        rusqlite::params![pending_projection.to_string(), old_ts.to_string()],
    )
    .expect("insert pending projection task");

    let recovered =
        super::recover_stale_running_tasks_on_startup(&db, 60).expect("recover stale running");

    assert!(recovered.is_empty());
    let (status, error_text): (String, Option<String>) = db
        .query_row(
            "SELECT status, error_text FROM tasks WHERE task_id = 'terminal-projection-pending'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query pending projection task");
    assert_eq!(status, "running");
    assert!(error_text.is_none());
}

fn paused_checkpoint_result(state: &str, next_check_after: i64, checkpoint_id: &str) -> String {
    json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": state,
            "source": "agent_loop_soft_budget",
            "resume_reason": "agent_loop_max_rounds",
            "next_check_after": next_check_after,
            "checkpoint_id": checkpoint_id,
            "can_poll": true,
            "can_cancel": true
        },
        "task_checkpoint": {
            "schema_version": 1,
            "checkpoint_id": checkpoint_id,
            "resume_entrypoint": "next_planner_round"
        }
    })
    .to_string()
}

#[test]
fn startup_recovery_preserves_paused_checkpoints_before_or_after_next_check() {
    let db = memory_tasks_db();
    let old_ts = 1_i64;
    let future_result = paused_checkpoint_result(
        "waiting",
        crate::now_ts_u64() as i64 + 3600,
        "checkpoint-future",
    );
    let due_result = paused_checkpoint_result("background", 1, "checkpoint-due");
    let needs_user_result = paused_checkpoint_result("needs_user", 1, "checkpoint-needs-user");
    for (task_id, result_json) in [
        ("running-old", None),
        ("waiting-future", Some(future_result)),
        ("background-due", Some(due_result)),
        ("needs-user", Some(needs_user_result)),
    ] {
        db.execute(
            "INSERT INTO tasks (task_id, status, result_json, error_text, created_at, updated_at)
             VALUES (?1, 'running', ?2, NULL, ?3, ?3)",
            rusqlite::params![task_id, result_json, old_ts.to_string()],
        )
        .expect("insert task");
    }

    let recovered =
        super::recover_stale_running_tasks_on_startup(&db, 60).expect("recover stale running");

    assert_eq!(recovered, vec!["running-old".to_string()]);
    let statuses = [
        "running-old",
        "waiting-future",
        "background-due",
        "needs-user",
    ]
    .into_iter()
    .map(|task_id| {
        db.query_row(
            "SELECT status FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| row.get::<_, String>(0),
        )
        .expect("query task status")
    })
    .collect::<Vec<_>>();
    assert_eq!(statuses, vec!["timeout", "running", "running", "running"]);
}

#[tokio::test]
async fn runtime_recovery_tick_materializes_due_checkpoint_without_nl_fields() {
    let state = state_with_runtime_tasks_table();
    let due = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "source": "agent_loop_soft_budget",
            "resume_reason": "await_user_input",
            "next_check_after": 1,
            "checkpoint_id": "ckpt-runtime"
        },
        "task_checkpoint": valid_checkpoint_json("ckpt-runtime", "await_user_input")
    });
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES ('runtime-due', 42, 7, 'test-key', 'ui', 'ask', ?1, 'running', ?2, NULL, '1', '1')",
        rusqlite::params![
            json!({"text": "checkpoint task"}).to_string(),
            due.to_string()
        ],
    )
    .expect("insert runtime due task");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES ('runtime-stale', 42, 7, 'test-key', 'ui', 'ask', ?1, 'running', NULL, NULL, '1', '1')",
        rusqlite::params![json!({"text": "stale task"}).to_string()],
    )
    .expect("insert runtime stale task");
    drop(db);

    super::runtime_support::maybe_recover_stale_running_tasks_runtime(&state)
        .await
        .expect("runtime recovery tick");

    let db = state.core.db.get().expect("get db");
    let (stale_status, stale_error): (String, Option<String>) = db
        .query_row(
            "SELECT status, error_text FROM tasks WHERE task_id = 'runtime-stale'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query runtime stale task");
    assert_eq!(stale_status, "timeout");
    assert!(stale_error
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty()));

    let (status, raw_result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = 'runtime-due'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query runtime due task");
    let result: serde_json::Value = serde_json::from_str(&raw_result).expect("parse result");
    let lifecycle = result
        .get("task_lifecycle")
        .and_then(serde_json::Value::as_object)
        .expect("task_lifecycle object");

    assert_eq!(status, "running");
    assert_eq!(lifecycle["state"], "needs_user");
    assert_eq!(lifecycle["checkpoint_id"], "ckpt-runtime");
    assert_eq!(
        lifecycle["resume_work_item"]["resume_directive"],
        "await_user_input"
    );
    assert_eq!(
        lifecycle["resume_executor"]["executor_state"],
        "awaiting_user"
    );
    assert_eq!(
        lifecycle["resume_executor"]["resume_directive"],
        "await_user_input"
    );
    assert!(lifecycle["resume_work_item"].get("text").is_none());
    assert!(lifecycle["resume_work_item"].get("error_text").is_none());
    assert!(lifecycle["resume_executor"].get("text").is_none());
    assert!(lifecycle["resume_executor"].get("error_text").is_none());
}

#[tokio::test]
async fn runtime_recovery_skips_current_worker_expired_lease() {
    let state = state_with_runtime_tasks_table();
    let now = crate::now_ts_u64() as i64;
    let worker_id = state.worker.worker_id.clone();
    {
        let db = state.core.db.get().expect("get db");
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, channel, kind, payload_json,
                status, result_json, error_text, created_at, updated_at,
                lease_owner, lease_expires_at, claim_attempt, claimed_at
            )
            VALUES ('runtime-current-worker-expired', 42, 7, 'test-key', 'ui', 'ask', ?1,
                    'running', NULL, NULL, ?2, ?2, ?3, ?4, 1, ?2)",
            rusqlite::params![
                json!({"text": "current worker long task"}).to_string(),
                now.to_string(),
                worker_id,
                now.saturating_sub(1),
            ],
        )
        .expect("insert current worker expired lease task");
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, channel, kind, payload_json,
                status, result_json, error_text, created_at, updated_at,
                lease_owner, lease_expires_at, claim_attempt, claimed_at
            )
            VALUES ('runtime-old-worker-expired', 42, 7, 'test-key', 'ui', 'ask', ?1,
                    'running', NULL, NULL, ?2, ?2, 'worker:old', ?3, 1, ?2)",
            rusqlite::params![
                json!({"text": "old worker task"}).to_string(),
                now.to_string(),
                now.saturating_sub(1),
            ],
        )
        .expect("insert old worker expired lease task");
    }

    super::runtime_support::maybe_recover_stale_running_tasks_runtime(&state)
        .await
        .expect("runtime recovery tick");

    let db = state.core.db.get().expect("get db");
    let current_status: String = db
        .query_row(
            "SELECT status FROM tasks WHERE task_id = 'runtime-current-worker-expired'",
            [],
            |row| row.get(0),
        )
        .expect("query current worker task");
    let (old_status, old_error): (String, Option<String>) = db
        .query_row(
            "SELECT status, error_text FROM tasks WHERE task_id = 'runtime-old-worker-expired'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query old worker task");

    assert_eq!(current_status, "running");
    assert_eq!(old_status, "timeout");
    assert_eq!(old_error.as_deref(), Some("worker_lease_expired"));
}

#[tokio::test]
async fn runtime_recovery_survives_file_backed_restart_without_duplicate_claim() {
    let temp = TempDirGuard::new("runtime_restart_checkpoint");
    let db_path = temp.path.join("tasks.sqlite");
    let first_state = file_backed_state_with_schema(&db_path);
    let mut checkpoint = valid_checkpoint_json("ckpt-restart", "await_user_input");
    checkpoint["completed_side_effect_refs"] = json!(["write_file:tmp/report.txt"]);
    let due = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "source": "agent_loop_soft_budget",
            "resume_reason": "await_user_input",
            "next_check_after": 1,
            "checkpoint_id": "ckpt-restart"
        },
        "task_checkpoint": checkpoint
    });
    {
        let db = first_state.core.db.get().expect("get first db");
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, channel, kind, payload_json,
                status, result_json, error_text, created_at, updated_at
            )
            VALUES ('runtime-restart', 42, 7, 'test-key', 'ui', 'ask', ?1, 'running', ?2, NULL, '1', '1')",
            rusqlite::params![
                json!({"text": "checkpoint task"}).to_string(),
                due.to_string()
            ],
        )
        .expect("insert file-backed due task");
    }
    drop(first_state);

    let restarted_state = file_backed_state_with_schema(&db_path);
    super::runtime_support::maybe_recover_stale_running_tasks_runtime(&restarted_state)
        .await
        .expect("runtime recovery after restart");

    let first_result = runtime_restart_result_json(&restarted_state);
    assert_eq!(first_result["task_lifecycle"]["state"], "needs_user");
    assert_eq!(
        first_result["task_lifecycle"]["checkpoint_id"],
        "ckpt-restart"
    );
    assert_eq!(
        first_result["task_checkpoint"]["completed_side_effect_refs"],
        json!(["write_file:tmp/report.txt"])
    );
    assert_eq!(
        first_result["task_lifecycle"]["resume_executor"]["executor_state"],
        "awaiting_user"
    );
    drop(restarted_state);

    let second_restarted_state = file_backed_state_with_schema(&db_path);
    super::runtime_support::maybe_recover_stale_running_tasks_runtime(&second_restarted_state)
        .await
        .expect("runtime recovery after second restart");
    let second_result = runtime_restart_result_json(&second_restarted_state);
    assert_eq!(second_result["task_lifecycle"]["state"], "needs_user");
    assert_eq!(
        second_result["task_checkpoint"]["completed_side_effect_refs"],
        json!(["write_file:tmp/report.txt"])
    );
    assert_eq!(
        second_result["task_lifecycle"]["resume_executor"]["executor_state"],
        "awaiting_user"
    );
}

#[tokio::test]
async fn runtime_recovery_reaches_terminal_state_after_file_backed_restart() {
    let temp = TempDirGuard::new("runtime_restart_terminal");
    let db_path = temp.path.join("tasks.sqlite");
    let first_state = file_backed_state_with_schema(&db_path);
    let mut checkpoint = valid_checkpoint_json("ckpt-terminal-restart", "verify_and_finalize");
    checkpoint["pending_action"] = json!({
        "final_result_json": {
            "status": "ok",
            "output": "RUSTCLAW_RESTART_TERMINAL_DONE"
        }
    });
    checkpoint["completed_side_effect_refs"] = json!(["write_file:tmp/report.txt"]);
    let due = json!({
        "task_lifecycle": {
            "schema_version": 1,
            "state": "waiting",
            "source": "agent_loop_soft_budget",
            "resume_reason": "verify_and_finalize",
            "next_check_after": 1,
            "checkpoint_id": "ckpt-terminal-restart"
        },
        "task_checkpoint": checkpoint
    });
    {
        let db = first_state.core.db.get().expect("get first db");
        db.execute(
            "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, channel, kind, payload_json,
                status, result_json, error_text, created_at, updated_at
            )
            VALUES ('runtime-terminal-restart', 42, 7, 'test-key', 'ui', 'ask', ?1, 'running', ?2, NULL, '1', '1')",
            rusqlite::params![
                json!({"text": "checkpoint task"}).to_string(),
                due.to_string()
            ],
        )
        .expect("insert file-backed terminal checkpoint task");
    }
    drop(first_state);

    let restarted_state = file_backed_state_with_schema(&db_path);
    super::runtime_support::maybe_recover_stale_running_tasks_runtime(&restarted_state)
        .await
        .expect("runtime recovery terminal projection after restart");
    let (status, result) = task_status_result_json(&restarted_state, "runtime-terminal-restart");
    assert_eq!(status, "succeeded");
    assert_eq!(result["output"], "RUSTCLAW_RESTART_TERMINAL_DONE");
    assert_eq!(result["task_lifecycle"]["state"], "succeeded");
    assert_eq!(
        result["task_lifecycle"]["checkpoint_id"],
        "ckpt-terminal-restart"
    );
    assert_eq!(
        result["task_lifecycle"]["terminal_executor_action"],
        "verify_and_finalize"
    );
    drop(restarted_state);

    let second_restarted_state = file_backed_state_with_schema(&db_path);
    super::runtime_support::maybe_recover_stale_running_tasks_runtime(&second_restarted_state)
        .await
        .expect("runtime recovery after terminal projection restart");
    let (second_status, second_result) =
        task_status_result_json(&second_restarted_state, "runtime-terminal-restart");
    assert_eq!(second_status, "succeeded");
    assert_eq!(second_result["output"], "RUSTCLAW_RESTART_TERMINAL_DONE");
    assert_eq!(
        second_result["task_lifecycle"]["terminal_executor_result_status"],
        "finalize_completed"
    );
}

fn runtime_restart_result_json(state: &crate::AppState) -> serde_json::Value {
    let (status, result) = task_status_result_json(state, "runtime-restart");
    assert_eq!(status, "running");
    result
}

fn task_status_result_json(state: &crate::AppState, task_id: &str) -> (String, serde_json::Value) {
    let db = state.core.db.get().expect("get restart db");
    let (status, raw_result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query runtime task");
    (
        status,
        serde_json::from_str(&raw_result).expect("parse restart result"),
    )
}

#[test]
fn paused_checkpoint_resume_work_item_is_machine_payload() {
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "ckpt-work-item".to_string(),
        boundary_context: json!({"route_gate_kind": "execute"}),
        last_successful_round: Some(2),
        last_successful_step: Some("step_2".to_string()),
        pending_action: None,
        observations: vec![json!({"step_id": "step_2", "status": "ok"})],
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: vec!["write_file:tmp/report.txt".to_string()],
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 2,
            step: 3,
            llm_calls: 4,
            tool_calls: 1,
            elapsed_ms: 500,
            llm_elapsed_ms: 500,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    let claimed = crate::repo::DuePausedCheckpointTask {
        task_id: "task-work-item".to_string(),
        lifecycle_state: "waiting".to_string(),
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        task_checkpoint: checkpoint.clone(),
        resume_entrypoint: "next_planner_round".to_string(),
        resume_wait_seconds: 0,
        completed_side_effect_count: 1,
        requires_idempotency_guard: true,
        checkpoint_resume_directive:
            crate::task_lifecycle::CheckpointResumeDirective::RunNextPlannerRound {
                checkpoint_id: checkpoint.checkpoint_id.clone(),
                completed_side_effect_count: 1,
                requires_idempotency_guard: true,
            },
        resume_directive: "run_next_planner_round".to_string(),
    };
    let mut loop_state = crate::agent_engine::LoopState::new(1);
    let seed_report = crate::agent_engine::seed_loop_state_for_agent_run(
        &mut loop_state,
        None,
        Some(&checkpoint),
    )
    .expect("checkpoint seed report");

    let work_item = super::runtime_support::build_paused_checkpoint_resume_work_item(
        &claimed,
        60,
        crate::task_lifecycle::ResumeTrigger::WorkerRecovery,
        seed_report,
    );
    let payload = work_item.to_machine_json();

    assert_eq!(payload["schema_version"], 1);
    assert_eq!(payload["executor_state"], "prepared");
    assert_eq!(payload["task_id"], "task-work-item");
    assert_eq!(payload["checkpoint_id"], "ckpt-work-item");
    assert_eq!(payload["resume_trigger"], "worker_recovery");
    assert_eq!(payload["resume_directive"], "run_next_planner_round");
    assert_eq!(
        payload["resume_directive_payload"]["requires_idempotency_guard"],
        true
    );
    assert_eq!(payload["seed_report"]["restored_round"], 2);
    assert_eq!(payload["seed_report"]["restored_step"], 3);
    assert_eq!(payload["seed_report"]["completed_side_effect_count"], 1);
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());

    let planner_decision = super::runtime_support::prepare_paused_checkpoint_resume_execution(
        &work_item,
        &claimed.checkpoint_resume_directive,
        1_000,
    );
    assert_eq!(planner_decision.executor_state, "ready_for_planner_resume");
    assert_eq!(planner_decision.lifecycle_state, Some("background"));
    assert_eq!(planner_decision.next_check_after, Some(1_000));
    assert_eq!(
        planner_decision.payload["resume_directive"],
        "run_next_planner_round"
    );
    assert_eq!(
        planner_decision.payload["resume_trigger"],
        "worker_recovery"
    );
    assert!(planner_decision.payload.get("text").is_none());
    assert!(planner_decision.payload.get("error_text").is_none());

    let poll_decision = super::runtime_support::prepare_paused_checkpoint_resume_execution(
        &work_item,
        &crate::task_lifecycle::CheckpointResumeDirective::PollAsyncJob {
            checkpoint_id: "ckpt-work-item".to_string(),
            job_id: "job-1".to_string(),
            adapter_kind: "media_job_poll".to_string(),
            poll_after_seconds: 7,
            expires_at: 2_000,
            cancel_ref: "cancel:job-1".to_string(),
            message_key: "tool.msg.job.running".to_string(),
        },
        1_000,
    );
    assert_eq!(poll_decision.executor_state, "poll_scheduled");
    assert_eq!(poll_decision.lifecycle_state, Some("background"));
    assert_eq!(poll_decision.next_check_after, Some(1_007));
    assert_eq!(poll_decision.payload["resume_trigger"], "worker_recovery");
    assert_eq!(poll_decision.payload["job_id"], "job-1");
    assert_eq!(poll_decision.payload["adapter_kind"], "media_job_poll");
    assert_eq!(poll_decision.payload["expires_at"], 2_000);

    let user_decision = super::runtime_support::prepare_paused_checkpoint_resume_execution(
        &work_item,
        &crate::task_lifecycle::CheckpointResumeDirective::AwaitUserInput {
            checkpoint_id: "ckpt-work-item".to_string(),
        },
        1_000,
    );
    assert_eq!(user_decision.executor_state, "awaiting_user");
    assert_eq!(user_decision.lifecycle_state, Some("needs_user"));
    assert_eq!(user_decision.next_check_after, None);
    assert_eq!(user_decision.payload["resume_trigger"], "worker_recovery");

    let finalize_decision = super::runtime_support::prepare_paused_checkpoint_resume_execution(
        &work_item,
        &crate::task_lifecycle::CheckpointResumeDirective::VerifyAndFinalize {
            checkpoint_id: "ckpt-work-item".to_string(),
            completed_side_effect_count: 1,
        },
        1_000,
    );
    assert_eq!(finalize_decision.executor_state, "ready_to_finalize");
    assert_eq!(finalize_decision.lifecycle_state, Some("background"));
    assert_eq!(finalize_decision.next_check_after, Some(1_000));
    assert_eq!(
        finalize_decision.payload["resume_trigger"],
        "worker_recovery"
    );
}

#[test]
fn claimed_paused_checkpoint_resume_executor_plans_machine_action() {
    let task = crate::ClaimedTask {
        task_id: "task-exec".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("u-ext".to_string()),
        external_chat_id: Some("c-ext".to_string()),
        kind: "ask".to_string(),
        payload_json: json!({"text": "continue"}).to_string(),
    };
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "ckpt-exec".to_string(),
        boundary_context: json!({"route_gate_kind": "execute"}),
        last_successful_round: Some(2),
        last_successful_step: Some("step_2".to_string()),
        pending_action: None,
        observations: vec![json!({"step_id": "step_2", "status": "ok"})],
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: vec!["write_file:tmp/report.txt".to_string()],
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 2,
            step: 3,
            llm_calls: 4,
            tool_calls: 1,
            elapsed_ms: 500,
            llm_elapsed_ms: 500,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    let claimed = crate::repo::ClaimedPausedCheckpointResumeExecutor {
        task: task.clone(),
        task_id: task.task_id.clone(),
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        previous_executor_state: "ready_for_planner_resume".to_string(),
        executor_state: "executing_planner_resume".to_string(),
        resume_trigger: "worker_recovery".to_string(),
        resume_directive: "run_next_planner_round".to_string(),
        lease_expires_at: 2_000,
        resume_executor: json!({
            "checkpoint_id": "ckpt-exec",
            "executor_state": "executing_planner_resume",
            "resume_directive": "run_next_planner_round",
            "resume_trigger": "worker_recovery",
            "requires_idempotency_guard": true
        }),
        resume_work_item: None,
        task_checkpoint: checkpoint.clone(),
    };

    let planner_plan =
        super::runtime_support::plan_claimed_paused_checkpoint_resume_execution(&claimed)
            .expect("planner resume plan");
    assert_eq!(planner_plan.task, task);
    assert_eq!(planner_plan.executor_action, "run_seeded_agent_loop");
    assert_eq!(planner_plan.executor_state, "executing_planner_resume");
    assert_eq!(planner_plan.resume_directive, "run_next_planner_round");
    assert_eq!(planner_plan.payload["task_kind"], "ask");
    assert_eq!(planner_plan.payload["task_channel"], "ui");
    assert_eq!(planner_plan.payload["task_payload_bytes"], 19);
    assert_eq!(planner_plan.payload["completed_side_effect_count"], 1);
    assert_eq!(planner_plan.payload["requires_idempotency_guard"], true);
    assert!(planner_plan.payload.get("text").is_none());
    assert!(planner_plan.payload.get("error_text").is_none());

    let mut poll_claimed = claimed.clone();
    poll_claimed.previous_executor_state = "poll_scheduled".to_string();
    poll_claimed.executor_state = "executing_async_poll".to_string();
    poll_claimed.resume_directive = "poll_async_job".to_string();
    poll_claimed.resume_executor = json!({
        "checkpoint_id": "ckpt-exec",
        "executor_state": "executing_async_poll",
        "resume_directive": "poll_async_job",
        "resume_trigger": "worker_recovery",
        "job_id": "job-1",
        "poll_after_seconds": 7,
        "expires_at": 2_500,
        "cancel_ref": "cancel:job-1",
        "message_key": "tool.msg.job.running"
    });
    let poll_plan =
        super::runtime_support::plan_claimed_paused_checkpoint_resume_execution(&poll_claimed)
            .expect("async poll plan");
    assert_eq!(poll_plan.executor_action, "poll_async_job");
    assert_eq!(poll_plan.payload["job_id"], "job-1");
    assert_eq!(poll_plan.payload["poll_after_seconds"], 7);
    assert_eq!(poll_plan.payload["expires_at"], 2_500);
    assert_eq!(poll_plan.payload["message_key"], "tool.msg.job.running");

    let mut finalize_claimed = claimed.clone();
    finalize_claimed.previous_executor_state = "ready_to_finalize".to_string();
    finalize_claimed.executor_state = "executing_finalize".to_string();
    finalize_claimed.resume_directive = "verify_and_finalize".to_string();
    let finalize_plan =
        super::runtime_support::plan_claimed_paused_checkpoint_resume_execution(&finalize_claimed)
            .expect("finalize plan");
    assert_eq!(finalize_plan.executor_action, "verify_and_finalize");

    let mut invalid_claimed = claimed;
    invalid_claimed.executor_state = "executing_async_poll".to_string();
    invalid_claimed.resume_directive = "run_next_planner_round".to_string();
    assert!(
        super::runtime_support::plan_claimed_paused_checkpoint_resume_execution(&invalid_claimed)
            .is_none(),
        "executor state and directive mismatch must not produce a replay plan"
    );
}

#[test]
fn planned_paused_checkpoint_resume_executor_handoff_is_machine_only() {
    let seeded_loop = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff",
        "executor_state": "executing_planner_resume",
        "executor_action": "run_seeded_agent_loop",
        "resume_directive": "run_next_planner_round"
    });
    let seeded_handoff =
        super::runtime_support::planned_paused_checkpoint_resume_executor_handoff(&seeded_loop)
            .expect("seeded loop handoff");
    assert_eq!(seeded_handoff.executor_action, "run_seeded_agent_loop");
    assert_eq!(
        seeded_handoff.executor_status,
        "seeded_loop_requires_provider_window"
    );
    assert_eq!(seeded_handoff.checkpoint_id, "ckpt-handoff");
    assert_eq!(
        seeded_handoff.payload["executor_status"],
        "seeded_loop_requires_provider_window"
    );
    assert!(seeded_handoff.payload.get("text").is_none());
    assert!(seeded_handoff.payload.get("error_text").is_none());

    let async_poll = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-poll",
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "resume_directive": "poll_async_job",
        "job_id": "job-1"
    });
    let poll_handoff =
        super::runtime_support::planned_paused_checkpoint_resume_executor_handoff(&async_poll)
            .expect("async poll handoff");
    assert_eq!(poll_handoff.executor_action, "poll_async_job");
    assert_eq!(poll_handoff.executor_status, "async_poll_adapter_pending");

    let finalize = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-finalize",
        "executor_state": "executing_finalize",
        "executor_action": "verify_and_finalize",
        "resume_directive": "verify_and_finalize"
    });
    let finalize_handoff =
        super::runtime_support::planned_paused_checkpoint_resume_executor_handoff(&finalize)
            .expect("finalize handoff");
    assert_eq!(finalize_handoff.executor_action, "verify_and_finalize");
    assert_eq!(
        finalize_handoff.executor_status,
        "checkpoint_finalize_executor_pending"
    );

    let invalid_text_plan = json!({
        "checkpoint_id": "ckpt-invalid",
        "executor_state": "executing_planner_resume",
        "executor_action": "run_seeded_agent_loop",
        "text": "not machine-only"
    });
    assert!(
        super::runtime_support::planned_paused_checkpoint_resume_executor_handoff(
            &invalid_text_plan,
        )
        .is_none(),
        "planned executor handoff must not accept user-visible text payloads"
    );

    let invalid_poll = json!({
        "checkpoint_id": "ckpt-invalid-poll",
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job"
    });
    assert!(
        super::runtime_support::planned_paused_checkpoint_resume_executor_handoff(&invalid_poll)
            .is_none(),
        "async poll handoff requires a machine job_id"
    );
}

#[test]
fn claimed_paused_checkpoint_resume_handoff_dispatch_is_machine_only() {
    let task = crate::ClaimedTask {
        task_id: "task-handoff-dispatch".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("u-ext".to_string()),
        external_chat_id: Some("c-ext".to_string()),
        kind: "ask".to_string(),
        payload_json: json!({"request_kind": "resume"}).to_string(),
    };
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "ckpt-handoff-dispatch".to_string(),
        boundary_context: json!({"route_gate_kind": "execute"}),
        last_successful_round: Some(2),
        last_successful_step: Some("step_2".to_string()),
        pending_action: None,
        observations: vec![json!({"step_id": "step_2", "status": "ok"})],
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: vec!["write_file:tmp/report.txt".to_string()],
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 2,
            step: 3,
            llm_calls: 4,
            tool_calls: 1,
            elapsed_ms: 500,
            llm_elapsed_ms: 500,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    let claimed = crate::repo::ClaimedHandoffPausedCheckpointResumeExecution {
        task: task.clone(),
        task_id: task.task_id.clone(),
        checkpoint_id: checkpoint.checkpoint_id.clone(),
        executor_state: "executing_planner_resume".to_string(),
        executor_action: "run_seeded_agent_loop".to_string(),
        executor_status: "seeded_loop_requires_provider_window".to_string(),
        resume_trigger: "worker_recovery".to_string(),
        resume_directive: "run_next_planner_round".to_string(),
        lease_expires_at: 2_000,
        handoff_claim_expires_at: 1_900,
        execution_plan: json!({
            "schema_version": 1,
            "checkpoint_id": "ckpt-handoff-dispatch",
            "executor_state": "executing_planner_resume",
            "executor_action": "run_seeded_agent_loop",
            "resume_directive": "run_next_planner_round",
            "requires_idempotency_guard": true
        }),
        handoff_payload: json!({
            "schema_version": 1,
            "checkpoint_id": "ckpt-handoff-dispatch",
            "executor_state": "executing_planner_resume",
            "executor_action": "run_seeded_agent_loop",
            "executor_status": "seeded_loop_requires_provider_window"
        }),
        handoff_claim: json!({
            "schema_version": 1,
            "checkpoint_id": "ckpt-handoff-dispatch",
            "executor_state": "executing_planner_resume",
            "executor_action": "run_seeded_agent_loop",
            "executor_status": "seeded_loop_requires_provider_window",
            "owner": "worker_recovery_handoff_executor",
            "expires_at": 1_900
        }),
        task_checkpoint: checkpoint.clone(),
    };

    let seeded_dispatch =
        super::runtime_support::dispatch_claimed_paused_checkpoint_resume_handoff(&claimed)
            .expect("seeded loop dispatch");
    assert_eq!(seeded_dispatch.task, task);
    assert_eq!(
        seeded_dispatch.dispatch_state,
        "ready_to_run_seeded_agent_loop"
    );
    assert_eq!(
        seeded_dispatch.payload["executor_status"],
        "seeded_loop_requires_provider_window"
    );
    assert_eq!(seeded_dispatch.payload["requires_idempotency_guard"], true);
    assert!(seeded_dispatch.payload.get("text").is_none());
    assert!(seeded_dispatch.payload.get("error_text").is_none());

    let mut poll_checkpoint = checkpoint.clone();
    poll_checkpoint.resume_entrypoint = crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob;
    let mut poll_claimed = claimed.clone();
    poll_claimed.task_checkpoint = poll_checkpoint;
    poll_claimed.executor_state = "executing_async_poll".to_string();
    poll_claimed.executor_action = "poll_async_job".to_string();
    poll_claimed.executor_status = "async_poll_adapter_pending".to_string();
    poll_claimed.resume_directive = "poll_async_job".to_string();
    poll_claimed.execution_plan = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff-dispatch",
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "resume_directive": "poll_async_job",
        "job_id": "job-1",
        "poll_after_seconds": 7,
        "expires_at": 2_500,
        "cancel_ref": "cancel:job-1",
        "message_key": "tool.msg.job.running"
    });
    poll_claimed.handoff_payload = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff-dispatch",
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "executor_status": "async_poll_adapter_pending"
    });
    poll_claimed.handoff_claim = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff-dispatch",
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "executor_status": "async_poll_adapter_pending",
        "owner": "worker_recovery_handoff_executor",
        "expires_at": 1_900
    });
    let poll_dispatch =
        super::runtime_support::dispatch_claimed_paused_checkpoint_resume_handoff(&poll_claimed)
            .expect("async poll dispatch");
    assert_eq!(poll_dispatch.dispatch_state, "ready_to_poll_async_job");
    assert_eq!(poll_dispatch.payload["job_id"], "job-1");
    assert_eq!(poll_dispatch.payload["poll_after_seconds"], 7);

    let mut finalize_checkpoint = checkpoint;
    finalize_checkpoint.resume_entrypoint =
        crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize;
    let mut finalize_claimed = claimed.clone();
    finalize_claimed.task_checkpoint = finalize_checkpoint;
    finalize_claimed.executor_state = "executing_finalize".to_string();
    finalize_claimed.executor_action = "verify_and_finalize".to_string();
    finalize_claimed.executor_status = "checkpoint_finalize_executor_pending".to_string();
    finalize_claimed.resume_directive = "verify_and_finalize".to_string();
    finalize_claimed.execution_plan = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff-dispatch",
        "executor_state": "executing_finalize",
        "executor_action": "verify_and_finalize",
        "resume_directive": "verify_and_finalize"
    });
    finalize_claimed.handoff_payload = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff-dispatch",
        "executor_state": "executing_finalize",
        "executor_action": "verify_and_finalize",
        "executor_status": "checkpoint_finalize_executor_pending"
    });
    finalize_claimed.handoff_claim = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-handoff-dispatch",
        "executor_state": "executing_finalize",
        "executor_action": "verify_and_finalize",
        "executor_status": "checkpoint_finalize_executor_pending",
        "owner": "worker_recovery_handoff_executor",
        "expires_at": 1_900
    });
    let finalize_dispatch =
        super::runtime_support::dispatch_claimed_paused_checkpoint_resume_handoff(
            &finalize_claimed,
        )
        .expect("finalize dispatch");
    assert_eq!(
        finalize_dispatch.dispatch_state,
        "ready_to_verify_and_finalize"
    );

    let mut invalid_entrypoint = claimed.clone();
    invalid_entrypoint.executor_action = "poll_async_job".to_string();
    invalid_entrypoint.executor_status = "async_poll_adapter_pending".to_string();
    invalid_entrypoint.resume_directive = "poll_async_job".to_string();
    invalid_entrypoint.execution_plan = json!({
        "checkpoint_id": "ckpt-handoff-dispatch",
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "resume_directive": "poll_async_job",
        "job_id": "job-1"
    });
    assert!(
        super::runtime_support::dispatch_claimed_paused_checkpoint_resume_handoff(
            &invalid_entrypoint,
        )
        .is_none(),
        "entrypoint/action mismatch must not dispatch"
    );

    let mut invalid_text = claimed;
    invalid_text.handoff_claim = json!({"text": "not machine-only"});
    assert!(
        super::runtime_support::dispatch_claimed_paused_checkpoint_resume_handoff(&invalid_text)
            .is_none(),
        "claimed handoff dispatch must reject user-visible text"
    );
}

#[test]
fn claimed_dispatch_result_payload_defers_only_supported_machine_states() {
    let task = crate::ClaimedTask {
        task_id: "task-dispatch-result".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("test-key".to_string()),
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: json!({"text": "continue"}).to_string(),
    };
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "ckpt-dispatch-result".to_string(),
        boundary_context: json!({"route_gate_kind": "execute"}),
        last_successful_round: Some(1),
        last_successful_step: Some("step_1".to_string()),
        pending_action: None,
        observations: Vec::new(),
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: vec!["write_file:tmp/report.txt".to_string()],
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 1,
            step: 2,
            llm_calls: 3,
            tool_calls: 1,
            elapsed_ms: 200,
            llm_elapsed_ms: 200,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    let claimed = crate::repo::ClaimedDispatchedPausedCheckpointResumeExecution {
        task,
        task_id: "task-dispatch-result".to_string(),
        checkpoint_id: "ckpt-dispatch-result".to_string(),
        executor_state: "executing_planner_resume".to_string(),
        executor_action: "run_seeded_agent_loop".to_string(),
        executor_status: "seeded_loop_requires_provider_window".to_string(),
        dispatch_state: "ready_to_run_seeded_agent_loop".to_string(),
        dispatch_execution_state: "claimed_to_run_seeded_agent_loop".to_string(),
        resume_trigger: "worker_recovery".to_string(),
        resume_directive: "run_next_planner_round".to_string(),
        lease_expires_at: 2_000,
        handoff_claim_expires_at: 1_900,
        dispatch_claim_expires_at: 1_850,
        execution_plan: json!({
            "schema_version": 1,
            "checkpoint_id": "ckpt-dispatch-result",
            "executor_state": "executing_planner_resume",
            "executor_action": "run_seeded_agent_loop",
            "resume_directive": "run_next_planner_round"
        }),
        dispatch_payload: json!({
            "schema_version": 1,
            "checkpoint_id": "ckpt-dispatch-result",
            "executor_state": "executing_planner_resume",
            "executor_action": "run_seeded_agent_loop",
            "executor_status": "seeded_loop_requires_provider_window",
            "dispatch_state": "ready_to_run_seeded_agent_loop"
        }),
        dispatch_claim: json!({
            "schema_version": 1,
            "checkpoint_id": "ckpt-dispatch-result",
            "executor_state": "executing_planner_resume",
            "executor_action": "run_seeded_agent_loop",
            "executor_status": "seeded_loop_requires_provider_window",
            "dispatch_state": "ready_to_run_seeded_agent_loop"
        }),
        task_checkpoint: checkpoint.clone(),
    };

    let payload = super::runtime_support::paused_checkpoint_resume_dispatch_result_payload(
        &claimed, 1_000, 60,
    )
    .expect("seeded loop deferred result");
    assert_eq!(payload["executor_result_status"], "seeded_loop_deferred");
    assert_eq!(
        payload["defer_reason_code"],
        "seeded_loop_executor_pending_integration"
    );
    assert_eq!(payload["next_check_after"], 1_060);
    assert_eq!(payload["completed_side_effect_count"], 1);
    assert!(payload.get("text").is_none());
    assert!(payload.get("error_text").is_none());

    let mut poll_claimed = claimed.clone();
    poll_claimed.executor_state = "executing_async_poll".to_string();
    poll_claimed.executor_action = "poll_async_job".to_string();
    poll_claimed.executor_status = "async_poll_adapter_pending".to_string();
    poll_claimed.dispatch_state = "ready_to_poll_async_job".to_string();
    poll_claimed.dispatch_execution_state = "claimed_to_poll_async_job".to_string();
    poll_claimed.resume_directive = "poll_async_job".to_string();
    poll_claimed.task_checkpoint.resume_entrypoint =
        crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob;
    poll_claimed.execution_plan = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-dispatch-result",
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "resume_directive": "poll_async_job",
        "job_id": "job-1",
        "poll_after_seconds": 7,
        "expires_at": 2_500,
        "cancel_ref": "cancel:job-1",
        "message_key": "tool.msg.job.running"
    });
    poll_claimed.dispatch_payload = json!({
        "schema_version": 1,
        "checkpoint_id": "ckpt-dispatch-result",
        "executor_state": "executing_async_poll",
        "executor_action": "poll_async_job",
        "executor_status": "async_poll_adapter_pending",
        "dispatch_state": "ready_to_poll_async_job"
    });
    let poll_payload = super::runtime_support::paused_checkpoint_resume_dispatch_result_payload(
        &poll_claimed,
        1_000,
        30,
    )
    .expect("async poll rescheduled result");
    assert_eq!(
        poll_payload["executor_result_status"],
        "async_poll_rescheduled"
    );
    assert_eq!(
        poll_payload["defer_reason_code"],
        "async_poll_adapter_pending"
    );
    assert_eq!(poll_payload["job_id"], "job-1");
    assert_eq!(poll_payload["next_check_after"], 1_030);
    assert!(poll_payload.get("text").is_none());
    assert!(poll_payload.get("error_text").is_none());

    let mut finalize_claimed = claimed;
    finalize_claimed.executor_state = "executing_finalize".to_string();
    finalize_claimed.executor_action = "verify_and_finalize".to_string();
    finalize_claimed.executor_status = "checkpoint_finalize_executor_pending".to_string();
    finalize_claimed.dispatch_state = "ready_to_verify_and_finalize".to_string();
    finalize_claimed.dispatch_execution_state = "claimed_to_verify_and_finalize".to_string();
    finalize_claimed.resume_directive = "verify_and_finalize".to_string();
    finalize_claimed.task_checkpoint.resume_entrypoint =
        crate::task_lifecycle::ResumeEntrypoint::VerifyAndFinalize;
    let finalize_payload =
        super::runtime_support::paused_checkpoint_resume_dispatch_result_payload(
            &finalize_claimed,
            1_000,
            30,
        )
        .expect("finalize dispatch terminal payload");
    assert_eq!(
        finalize_payload["executor_result_status"],
        "finalize_failed"
    );
    assert_eq!(
        finalize_payload["error_code"],
        "checkpoint_finalize_missing_final_result"
    );
    assert!(finalize_payload.get("text").is_none());
    assert!(finalize_payload.get("error_text").is_none());
}

#[test]
fn sync_recovery_dispatch_claim_includes_concrete_terminal_executors() {
    assert!(
        super::runtime_support::sync_recovery_can_claim_dispatch_executor("run_seeded_agent_loop")
    );
    assert!(super::runtime_support::sync_recovery_can_claim_dispatch_executor("poll_async_job"));
    assert!(
        super::runtime_support::sync_recovery_can_claim_dispatch_executor("verify_and_finalize"),
        "sync recovery can claim finalize once the concrete terminal payload contract exists"
    );
}

#[test]
fn claimed_dispatch_result_reschedule_projection_payload_is_machine_only() {
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "ckpt-projection".to_string(),
        boundary_context: json!({"route_gate_kind": "execute"}),
        last_successful_round: Some(1),
        last_successful_step: Some("step_1".to_string()),
        pending_action: None,
        observations: Vec::new(),
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: Vec::new(),
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 1,
            step: 1,
            llm_calls: 2,
            tool_calls: 0,
            elapsed_ms: 100,
            llm_elapsed_ms: 100,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    let claimed = crate::repo::ClaimedPausedCheckpointResumeDispatchResult {
        task: crate::ClaimedTask {
            task_id: "task-projection".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: Some("test-key".to_string()),
            channel: "telegram".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        },
        task_id: "task-projection".to_string(),
        checkpoint_id: "ckpt-projection".to_string(),
        executor_state: "executing_planner_resume".to_string(),
        executor_action: "run_seeded_agent_loop".to_string(),
        executor_status: "seeded_loop_requires_provider_window".to_string(),
        dispatch_state: "ready_to_run_seeded_agent_loop".to_string(),
        executor_result_status: "seeded_loop_deferred".to_string(),
        result_projection_state: "project_seeded_loop_deferred".to_string(),
        recorded_at: 100,
        result_projection_claim_expires_at: 160,
        execution_result_payload: json!({
            "checkpoint_id": "ckpt-projection",
            "executor_state": "executing_planner_resume",
            "executor_action": "run_seeded_agent_loop",
            "executor_status": "seeded_loop_requires_provider_window",
            "dispatch_state": "ready_to_run_seeded_agent_loop",
            "executor_result_status": "seeded_loop_deferred",
            "retry_after_seconds": 60
        }),
        result_projection_claim: json!({"owner": "worker_recovery_result_projector"}),
        task_checkpoint: checkpoint,
    };

    let payload =
        super::runtime_support::paused_checkpoint_resume_reschedule_projection_payload(&claimed)
            .expect("reschedule projection payload");

    assert_eq!(payload["task_id"], "task-projection");
    assert_eq!(payload["checkpoint_id"], "ckpt-projection");
    assert_eq!(payload["executor_action"], "run_seeded_agent_loop");
    assert_eq!(payload["executor_result_status"], "seeded_loop_deferred");
    assert_eq!(
        payload["result_projection_state"],
        "project_seeded_loop_deferred"
    );
    assert_eq!(payload["retry_after_seconds"], 60);
    assert!(
        payload.get("text").is_none() && payload.get("error_text").is_none(),
        "reschedule projection payload must remain machine-only"
    );

    let mut completed = claimed;
    completed.executor_result_status = "seeded_loop_completed".to_string();
    completed.result_projection_state = "project_seeded_loop_completed".to_string();
    completed.execution_result_payload["executor_result_status"] = json!("seeded_loop_completed");
    assert!(
        super::runtime_support::paused_checkpoint_resume_reschedule_projection_payload(&completed)
            .is_none(),
        "non-reschedule result projection requires a separate final-state contract"
    );
}

#[test]
fn schedule_notify_observation_marks_delivery_failure() {
    let observation = super::schedule_notify_observation(&super::ScheduleNotifyOutcome {
        job_id: "job-1".to_string(),
        channel: "telegram".to_string(),
        runtime_channel: "telegram".to_string(),
        task_success: true,
        delivered: false,
        error_text: Some("telegram bot token is empty".to_string()),
    });

    assert_eq!(
        observation.get("source").and_then(|value| value.as_str()),
        Some("schedule_notify")
    );
    assert_eq!(
        observation
            .get("execution_surface_owner")
            .and_then(|value| value.as_str()),
        Some("delivery_boundary")
    );
    assert_eq!(
        observation.get("status").and_then(|value| value.as_str()),
        Some("error")
    );
    assert_eq!(
        observation
            .get("error_kind")
            .and_then(|value| value.as_str()),
        Some("channel_send_failed")
    );
    assert_eq!(
        observation
            .get("failure_attribution")
            .and_then(|value| value.as_str()),
        Some("delivery_error")
    );
}
